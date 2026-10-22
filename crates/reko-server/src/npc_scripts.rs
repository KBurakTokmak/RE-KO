//! Reference-script NPC menus with atomic item, currency and quest updates.
use crate::{combat, inventory, lobby::Lobby, shops::Merchant, starter, world_entry::Session};
use mlua::{HookTriggers, Lua, LuaOptions, StdLib, Variadic, VmState};
use rusqlite::{Connection, params};
use std::{cell::RefCell, collections::HashMap, net::SocketAddr};
use tokio::net::TcpStream;

#[derive(Clone, Default)]
pub(crate) struct Context {
    pub npc: u16,
    pub helper: u32,
    pub file: String,
    pub choices: [i32; 10],
}
#[derive(Clone)]
struct Helper {
    id: u32,
    npc: u16,
    event: i32,
    complete: i32,
    exchange: u32,
    talk: i32,
    kind: u8,
    flag: u16,
    status: u8,
    level: u8,
    experience: u32,
    class: u8,
    nation: u8,
    file: String,
}
fn number(v: &serde_json::Value, key: &str) -> i64 {
    v[key]
        .as_str()
        .and_then(|s| s.parse().ok())
        .or_else(|| v[key].as_i64())
        .unwrap_or(0)
}
fn helpers(db: &Connection) -> anyhow::Result<Vec<Helper>> {
    let mut q=db.prepare("SELECT rule_id,data FROM world_reference_rules WHERE table_name='QUEST_HELPER' ORDER BY rule_id")?;
    let mut out = Vec::new();
    for row in q.query_map([], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, String>(1)?)))? {
        let (id, text) = row?;
        let v: serde_json::Value = serde_json::from_str(&text)?;
        out.push(Helper {
            id,
            npc: number(&v, "sNpcId") as u16,
            event: number(&v, "nEventTriggerIndex") as i32,
            complete: number(&v, "nEventCompleteIndex") as i32,
            exchange: number(&v, "nExchangeIndex") as u32,
            talk: number(&v, "nEventTalkIndex") as i32,
            kind: number(&v, "bQuestType") as u8,
            flag: number(&v, "sEventDataIndex") as u16,
            status: number(&v, "bEventStatus") as u8,
            level: number(&v, "bLevel") as u8,
            experience: number(&v, "nExp") as u32,
            class: number(&v, "bClass") as u8,
            nation: number(&v, "bNation") as u8,
            file: v["strLuaFilename"]
                .as_str()
                .unwrap_or_default()
                .trim()
                .to_string(),
        });
    }
    Ok(out)
}
fn eligible(h: &Helper, s: &Session) -> bool {
    let job = match s.character.class % 100 {
        1 | 5 | 6 => 1,
        2 | 7 | 8 => 2,
        3 | 9 | 10 => 3,
        4 | 11 | 12 => 4,
        _ => 0,
    };
    (matches!(h.nation, 0 | 3) || h.nation == s.character.nation())
        && (h.class == 5 || h.class == 0 || h.class == job)
        && (h.level < s.progress.level
            || (h.level == s.progress.level && h.experience <= s.rewards.experience))
}
fn open_state(status: u8, state: u8) -> bool {
    match status {
        0 | 4 | 255 => matches!(state, 0 | 4),
        1 | 3 => matches!(state, 1 | 3),
        2 => state == 2,
        _ => false,
    }
}
fn entry_helper<'a>(all: &'a [Helper], h: &'a Helper) -> &'a Helper {
    if h.status == 255 && matches!(h.npc, 11510 | 21510) && matches!(h.flag, 603 | 605) {
        all.iter()
            .find(|other| {
                other.npc == h.npc
                    && other.flag == h.flag
                    && other.class == h.class
                    && other.nation == h.nation
                    && other.status == 0
            })
            .unwrap_or(h)
    } else {
        h
    }
}

fn database_quest_script(all: &[Helper], context: &Context) -> Option<Vec<u8>> {
    let h = all.iter().find(|h| h.id == context.helper)?;
    if h.npc == 18004 && (h.flag == 406 || h.flag == 0 && h.event == 100) {
        let done = all
            .iter()
            .find(|other| other.npc == h.npc && other.flag == 406 && other.status == 2)?;
        return Some(
            format!(
                r#"
local class=CheckClass(UID)
local status=GetQuestStatus(UID,406)
if EVENT==2147483001 then
    if CheckLevel(UID)>=10 and class>=1 and class<=4 and status~=2 then
        GoldLose(UID,GetNoviceCost(UID)); PromoteUserNovice(UID); SaveEvent(UID,{done});
        local message=4066
        if CheckNation(UID)==2 then message=4067 end
        SelectMsg(UID,2,406,message,18004,4064,2147483003)
    end
elseif EVENT~=2147483003 then
    if CheckLevel(UID)<10 then
        SelectMsg(UID,2,406,4069,18004,10,2147483003)
    elseif class<1 or class>4 or status==2 then
        SelectMsg(UID,2,406,4068,18004,10,2147483003)
    elseif HowmuchItem(UID,900000000)<GetNoviceCost(UID) then
        SelectMsg(UID,2,406,4065,18004,10,2147483003)
    else
        SelectMsg(UID,2,406,4064,18004,4062,2147483001,4063,2147483003)
    end
end
"#,
                done = done.id
            )
            .into_bytes(),
        );
    }
    if h.flag == 0 || h.kind != 1 || h.exchange == 0 || h.talk <= 0 {
        return None;
    }
    let active = all.iter().find(|other| {
        other.flag == h.flag
            && other.npc == h.npc
            && other.class == h.class
            && other.nation == h.nation
            && other.status == 1
    })?;
    let done = all.iter().find(|other| {
        other.flag == h.flag
            && other.npc == h.npc
            && other.class == h.class
            && other.nation == h.nation
            && other.status == 2
    })?;
    let suppression = matches!(h.npc, 11510 | 21510) && matches!(h.flag, 603 | 605);
    let enter = if suppression {
        "EnterSuppression(UID)"
    } else {
        ""
    };
    let incomplete = if suppression {
        format!(
            "SelectMsg(UID,4,{},{},{},22,2147483004,23,2147483003)",
            h.flag, h.talk, h.npc
        )
    } else {
        format!(
            "SelectMsg(UID,2,{},{},{},10,2147483003)",
            h.flag, h.talk, h.npc
        )
    };
    // Menu events remain scoped to the server's offered choices.
    Some(
        format!(
            r#"
local status=GetQuestStatus(UID,{flag})
if EVENT==2147483001 then
    if status==0 or status==4 then SaveEvent(UID,{active}); {enter}; end
elseif EVENT==2147483004 then
    if status==1 or status==3 then {enter}; end
elseif EVENT==2147483002 then
    if (status==1 or status==3) and CheckQuestExchange(UID,{exchange}) then
        RunQuestExchange(UID,{exchange}); SaveEvent(UID,{done});
    end
elseif EVENT~=2147483003 then
    if status==0 or status==4 then
        SelectMsg(UID,4,{flag},{talk},{npc},22,2147483001,23,2147483003)
    elseif status==1 or status==3 then
        if CheckQuestExchange(UID,{exchange}) then
            SelectMsg(UID,4,{flag},{talk},{npc},4006,2147483002,23,2147483003)
        else
            {incomplete}
        end
    else
        SelectMsg(UID,2,{flag},{talk},{npc},10,2147483003)
    end
end
"#,
            flag = h.flag,
            active = active.id,
            done = done.id,
            exchange = h.exchange,
            talk = h.talk,
            npc = h.npc,
            enter = enter,
            incomplete = incomplete
        )
        .into_bytes(),
    )
}
struct Changes {
    travel: Option<crate::movement::Position>,
    class: Option<u16>,
    rewards: crate::quest_rewards::State,
    level_state: Option<(combat::Stats, combat::Stats, crate::skills::State)>,
    bag: [starter::Item; 28],
    coins: u32,
    quests: HashMap<u16, u8>,
    packets: Vec<Vec<u8>>,
    context: Context,
}
impl Changes {
    fn count(&self, id: u32) -> u32 {
        if id == 900000000 {
            self.coins
        } else {
            self.bag
                .iter()
                .filter(|i| i.id == id)
                .map(|i| u32::from(i.count))
                .sum()
        }
    }
    fn take(&mut self, id: u32, quantity: u32) -> anyhow::Result<()> {
        anyhow::ensure!(
            quantity > 0 && self.count(id) >= quantity,
            "missing required item"
        );
        if id == 900000000 {
            self.coins -= quantity;
            return Ok(());
        }
        let mut left = quantity;
        for item in self.bag.iter_mut().filter(|i| i.id == id) {
            let n = left.min(u32::from(item.count));
            item.count -= n as u16;
            left -= n;
            if item.count == 0 {
                *item = starter::Item::default();
            }
            if left == 0 {
                break;
            }
        }
        Ok(())
    }
    fn give(&mut self, c: &combat::Catalog, id: u32, mut quantity: u32) -> anyhow::Result<()> {
        anyhow::ensure!(quantity > 0, "invalid item quantity");
        // Skill unlocks are represented by completing the quest flag.
        if id == 900007000 {
            return Ok(());
        }
        if self.rewards.gain(id, quantity)? {
            return Ok(());
        }
        if id == 900000000 {
            self.coins = self
                .coins
                .checked_add(quantity)
                .filter(|n| *n <= 2100000000)
                .ok_or_else(|| anyhow::anyhow!("coin limit"))?;
            return Ok(());
        }
        let (row, ext) = c
            .item(id)
            .ok_or_else(|| anyhow::anyhow!("reward absent from original client"))?;
        let ext = ext.ok_or_else(|| anyhow::anyhow!("unknown reward extension"))?;
        let stack = if combat::integer(row, 23) != 0 {
            9999
        } else {
            1
        };
        let durability =
            (combat::integer(row, 19) + combat::integer(ext, 12)).clamp(0, 65535) as u16;
        for item in &mut self.bag {
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
                return Ok(());
            }
        }
        anyhow::bail!("inventory full")
    }
}
type ExchangeResult = ([starter::Item; 28], u32, crate::quest_rewards::State);
fn objectives_met(
    db: &Connection,
    account: &str,
    slot: u8,
    state: &Changes,
    id: u32,
) -> anyhow::Result<bool> {
    let text: String = db.query_row(
        "SELECT data FROM world_reference_rules WHERE table_name='ITEM_EXCHANGE' AND rule_id=?1",
        [id],
        |r| r.get(0),
    )?;
    let row: serde_json::Value = serde_json::from_str(&text)?;
    for i in 1..=11 {
        let item = number(&row, &format!("nOriginItemNum{i}")) as u32;
        let count = number(&row, &format!("nOriginItemCount{i}")) as u32;
        if item == 0 {
            continue;
        }
        if count == 0
            || if item == 900005000 {
                !crate::quests::fulfilled(db, account, slot, &state.quests, count as u16)?
            } else {
                state.count(item) < count
            }
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn check_progress(
    lobby: &Lobby,
    account: &str,
    s: &Session,
    p: &[u8],
) -> anyhow::Result<Vec<Vec<u8>>> {
    let db = lobby.db.lock().unwrap();
    let tx = db.unchecked_transaction()?;
    let id = u32::from_le_bytes(p[2..6].try_into()?);
    let h = helpers(&tx)?
        .into_iter()
        .find(|h| h.id == id && h.flag > 0 && h.kind == 1 && h.exchange > 0 && eligible(h, s))
        .ok_or_else(|| anyhow::anyhow!("invalid quest progress helper"))?;
    let quests = tx
        .prepare("SELECT quest,state FROM character_quests WHERE account=?1 AND slot=?2")?
        .query_map(params![account, s.character.slot], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?
        .collect::<rusqlite::Result<HashMap<u16, u8>>>()?;
    let current = quests.get(&h.flag).copied().unwrap_or(0);
    anyhow::ensure!(matches!(current, 1 | 3), "quest not active");
    let state = Changes {
        travel: None,
        class: None,
        level_state: None,
        rewards: crate::quest_rewards::State::new(&tx, account, s)?,
        bag: starter::inventory(&tx, account, s.character.slot)?,
        coins: starter::coins(&tx, account, s.character.slot)?,
        quests,
        packets: vec![],
        context: Context::default(),
    };
    let next = if objectives_met(&tx, account, s.character.slot, &state, h.exchange)? {
        3
    } else {
        1
    };
    let mut packets = Vec::new();
    if current != next {
        tx.execute(
            "UPDATE character_quests SET state=?4 WHERE account=?1 AND slot=?2 AND quest=?3",
            params![account, s.character.slot, h.flag, next],
        )?;
        let mut packet = vec![0x64, 2];
        packet.extend(h.flag.to_le_bytes());
        packet.push(next);
        packets.push(packet);
    }
    tx.commit()?;
    Ok(packets)
}
fn exchange(
    db: &Connection,
    owner: (&str, u8),
    c: &combat::Catalog,
    state: &Changes,
    id: u32,
    preview: bool,
    selection: Option<i8>,
) -> anyhow::Result<ExchangeResult> {
    let text: String = db.query_row(
        "SELECT data FROM world_reference_rules WHERE table_name='ITEM_EXCHANGE' AND rule_id=?1",
        [id],
        |r| r.get(0),
    )?;
    let v: serde_json::Value = serde_json::from_str(&text)?;
    let mut next = Changes {
        travel: None,
        rewards: state.rewards.clone(),
        level_state: None,
        class: None,
        bag: state.bag,
        coins: state.coins,
        quests: HashMap::new(),
        packets: Vec::new(),
        context: Context::default(),
    };
    let kind = number(&v, "bRandomFlag");
    anyhow::ensure!((0..=101).contains(&kind), "unsupported special exchange");
    for i in 1..=11 {
        let id = number(&v, &format!("nOriginItemNum{i}")) as u32;
        let count = number(&v, &format!("nOriginItemCount{i}")) as u32;
        if id != 0 {
            if id == 900005000 {
                anyhow::ensure!(
                    crate::quests::fulfilled(db, owner.0, owner.1, &state.quests, count as u16)?,
                    "monster objective incomplete"
                );
            } else {
                next.take(id, count)?;
            }
        }
    }
    let rewards: Vec<_> = (1..=5)
        .map(|i| {
            (
                number(&v, &format!("nExchangeItemNum{i}")) as u32,
                number(&v, &format!("nExchangeItemCount{i}")) as u32,
            )
        })
        .collect();
    let mut seed = crate::skills::now_ms()
        .wrapping_mul(6364136223846793005)
        .wrapping_add(u64::from(id));
    if let (12, Some(selection)) = (kind, selection) {
        if preview && selection < 0 {
            let possible = rewards
                .iter()
                .filter(|(id, count)| *id != 0 && *count > 0)
                .any(|&(id, count)| {
                    let mut candidate = Changes {
                        travel: None,
                        rewards: next.rewards.clone(),
                        level_state: None,
                        class: None,
                        bag: next.bag,
                        coins: next.coins,
                        quests: HashMap::new(),
                        packets: Vec::new(),
                        context: Context::default(),
                    };
                    candidate.give(c, id, count).is_ok()
                });
            anyhow::ensure!(
                possible,
                "no compatible quest reward or free inventory slot"
            );
        } else {
            let &(id, count) = rewards
                .get(usize::try_from(selection)?)
                .filter(|(id, count)| *id != 0 && *count > 0)
                .ok_or_else(|| anyhow::anyhow!("invalid selected quest reward"))?;
            next.give(c, id, count)?;
        }
    } else if matches!(kind, 0 | 10 | 11 | 12) {
        for &(id, count) in &rewards {
            if id != 0 {
                next.give(c, id, count)?;
            }
        }
    } else if preview {
        for &(id, count) in &rewards {
            if id != 0 && count != 0 {
                let mut possible = Changes {
                    travel: None,
                    rewards: next.rewards.clone(),
                    level_state: None,
                    class: None,
                    bag: next.bag,
                    coins: next.coins,
                    quests: HashMap::new(),
                    packets: Vec::new(),
                    context: Context::default(),
                };
                possible.give(c, id, if kind == 101 { 1 } else { count })?;
            }
        }
    } else {
        seed ^= seed >> 29;
        let index = if kind == 101 {
            let sum: u32 = rewards.iter().map(|r| r.1).sum();
            anyhow::ensure!(sum <= 10000, "invalid exchange weights");
            let roll = (seed % 10000) as u32;
            let mut total = 0;
            let mut selected = 0;
            for (i, r) in rewards.iter().enumerate() {
                total += r.1;
                if roll < total {
                    selected = i;
                    break;
                }
            }
            selected
        } else {
            ((seed % ((kind as u64) * 1000 + 1)) / 1000).min(4) as usize
        };
        let (id, count) = rewards[index];
        if id != 0 {
            next.give(c, id, if kind == 101 { 1 } else { count })?;
        }
    }
    Ok((next.bag, next.coins, next.rewards))
}
fn execute(
    lobby: &Lobby,
    account: &str,
    s: &Session,
    merchant: &Merchant,
    context: Context,
    action: (i32, i8),
    script: &[u8],
) -> anyhow::Result<Changes> {
    let db = lobby.db.lock().unwrap();
    let tx = db.unchecked_transaction()?;
    let all = helpers(&tx)?;
    let abyss_trigger = crate::abyss::trigger(&tx, merchant, s, &context.file)?;
    let bifrost_script = crate::bifrost::script(merchant, s, &context.file)?;
    let mut abyss_hub = abyss_trigger.map(|_| lobby.social.lock().unwrap());
    let party = abyss_hub.as_ref().map_or_else(Vec::new, |hub| {
        hub.group(s.player_id)
            .into_iter()
            .filter(|id| *id != s.player_id)
            .filter_map(|id| hub.players.get(&id).cloned())
            .collect::<Vec<_>>()
    });
    let party_items = RefCell::new(Vec::<(u16, Vec<(usize, starter::Item)>)>::new());
    let party_travel = RefCell::new(None::<crate::movement::Position>);
    let mut context = context;
    let menu_reply = context.choices.iter().any(|e| *e >= 0);
    context.choices = [-1; 10];
    let quests = tx
        .prepare("SELECT quest,state FROM character_quests WHERE account=?1 AND slot=?2")?
        .query_map(params![account, s.character.slot], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?
        .collect::<rusqlite::Result<HashMap<_, _>>>()?;
    let state = RefCell::new(Changes {
        travel: None,
        rewards: crate::quest_rewards::State::new(&tx, account, s)?,
        level_state: None,
        class: None,
        bag: starter::inventory(&tx, account, s.character.slot)?,
        coins: starter::coins(&tx, account, s.character.slot)?,
        quests,
        packets: Vec::new(),
        context,
    });
    let before = (
        state.borrow().bag,
        state.borrow().coins,
        state.borrow().quests.clone(),
        state.borrow().rewards.clone(),
    );
    let lua = Lua::new_with(
        StdLib::TABLE | StdLib::STRING | StdLib::MATH,
        LuaOptions::default(),
    )?;
    lua.set_memory_limit(4 * 1024 * 1024)?;
    let instructions = std::cell::Cell::new(0u32);
    lua.set_hook(
        HookTriggers::new().every_nth_instruction(1000),
        move |_, _| {
            let n = instructions.get() + 1000;
            instructions.set(n);
            if n > 200000 {
                Err(mlua::Error::runtime("NPC script instruction limit"))
            } else {
                Ok(VmState::Continue)
            }
        },
    )?;
    for key in ["dofile", "loadfile", "load", "require", "collectgarbage"] {
        lua.globals().set(key, mlua::Value::Nil)?;
    }
    lua.globals().set("UID", s.player_id)?;
    lua.globals().set("EVENT", action.0)?;
    lua.globals().set("STEP", action.1)?;
    let ready = || -> mlua::Result<()> {
        for player in &party {
            if !player.s.entered
                || player.s.zone_loading
                || player.s.skills.hp == 0
                || !crate::social::near(s.position, player.s.position, 50)
                || player
                    .s
                    .skills
                    .buffs
                    .get(&22)
                    .is_some_and(|b| crate::skills::now_ms() < b.until)
                || abyss_hub
                    .as_ref()
                    .unwrap()
                    .transfers
                    .contains_key(&player.s.player_id)
            {
                return Err(mlua::Error::runtime(
                    "All party members must be alive, nearby and available for Abyss travel.",
                ));
            }
        }
        Ok(())
    };
    lua.scope(|scope| {
        macro_rules! function {
            ($name:literal,$f:expr) => {
                lua.globals().set($name, scope.create_function($f)?)?
            };
        }
        function!("CheckLevel", |_, _: Variadic<i64>| Ok(s.progress.level));
        function!("GetNoviceCost", |_, _: Variadic<i64>| {
            let text:String=tx.query_row("SELECT data FROM world_reference_rules WHERE table_name='ITEM_EXCHANGE' AND rule_id=460",[],|r|r.get(0)).map_err(mlua::Error::external)?;
            let row:serde_json::Value=serde_json::from_str(&text).map_err(mlua::Error::external)?;
            if number(&row,"nOriginItemNum1")!=900000000 || number(&row,"nExchangeItemNum1")!=900006000 || number(&row,"nExchangeItemCount1")!=1 || (2..=11).any(|i|number(&row,&format!("nOriginItemNum{i}"))!=0) || (2..=5).any(|i|number(&row,&format!("nExchangeItemNum{i}"))!=0) { return Err(mlua::Error::runtime("invalid original novice exchange")); }
            let cost=number(&row,"nOriginItemCount1");
            if cost<=0 {return Err(mlua::Error::runtime("invalid novice cost"));}
            Ok(cost)
        });
        function!("PromoteUserNovice", |_, _: Variadic<i64>| {
            let base=s.character.class%100;
            let mut st=state.borrow_mut();
            if merchant.prototype!=18004 || s.progress.level<10 || !(1..=4).contains(&base) || st.class.is_some() || st.quests.get(&406)==Some(&2) {return Err(mlua::Error::runtime("first job change unavailable"));}
            st.class=Some(s.character.class/100*100+base*2+3);
            Ok(true)
        });
        function!("CheckClass", |_, _: Variadic<i64>| Ok(
            s.character.class % 100
        ));
        function!("CheckNation", |_, _: Variadic<i64>| Ok(s
            .character
            .nation()));
        function!("GetZoneID", |_, _: Variadic<i64>| Ok(s.position.zone));
        if let Some(trigger)=abyss_trigger {
            function!("GetEventTrigger",move |_,_:Variadic<i64>|Ok(trigger));
            function!("ZoneChangeParty",|_,a:Variadic<i64>| {
                ready()?;
                let destination=crate::abyss::destination(&tx,s,&a).map_err(mlua::Error::external)?;
                let mut st=state.borrow_mut();
                if st.travel.is_some() {return Err(mlua::Error::runtime("Abyss travel already selected"));}
                for player in &party {
                    crate::movement::save(&tx,&player.account,player.s.character.slot,destination).map_err(mlua::Error::external)?;
                }
                st.travel=Some(destination);
                *party_travel.borrow_mut()=Some(destination);
                Ok(())
            });
            function!("RobAllItemParty",|_,a:Variadic<i64>| {
                ready()?;
                if a.len()!=2 || a[0]!=i64::from(s.player_id) || a[1]!=379080000 || !party_items.borrow().is_empty() {
                    return Err(mlua::Error::runtime("Invalid Abyss party key request"));
                }
                state.borrow_mut().take(379080000,1).map_err(mlua::Error::external)?;
                for player in &party {
                    let changed=inventory::consume(&tx,&player.account,player.s.character.slot,379080000,1).map_err(mlua::Error::external)?;
                    party_items.borrow_mut().push((player.s.player_id,changed));
                }
                Ok(true)
            });
        }
        if abyss_trigger.is_some() || bifrost_script {
            function!("ZoneChange",|_,a:Variadic<i64>| {
                let destination=if bifrost_script {crate::bifrost::destination(&tx,s,&a)}
                    else {crate::abyss::destination(&tx,s,&a)}.map_err(mlua::Error::external)?;
                let mut st=state.borrow_mut();
                if st.travel.is_some() {return Err(mlua::Error::runtime("Travel already selected"));}
                st.travel=Some(destination);
                Ok(())
            });
        }
        if bifrost_script {
            function!("CheckBeefRoastVictory",|_,a:Variadic<i64>| {
                if a.as_slice()!=[i64::from(s.player_id)] {return Err(mlua::Error::runtime("Invalid Bifrost owner"));}
                crate::bifrost::victory(&tx).map_err(mlua::Error::external)
            });
        }
        function!("EnterSuppression", |_, _: Variadic<i64>| {
            let mut st = state.borrow_mut();
            let helper = all.iter().find(|h| h.id == st.context.helper)
                .ok_or_else(|| mlua::Error::runtime("invalid suppression helper"))?;
            if !matches!(helper.npc,11510|21510) || !matches!(helper.flag,603|605)
                || merchant.prototype != helper.npc || !eligible(helper,s)
                || s.position.zone != s.character.nation() || s.zone_loading
                || !matches!(st.quests.get(&helper.flag),Some(1|3)) || st.travel.is_some() {
                return Err(mlua::Error::runtime("suppression entry unavailable"));
            }
            let zone = if helper.flag == 603 {82} else {83};
            if lobby.monsters.as_ref().and_then(|w|w.suppression_deadline(zone)).is_some_and(|end|crate::skills::now_ms()>=end) {
                return Err(mlua::Error::runtime("This shared suppression run has ended."));
            }
            let (x,z):(u16,u16) = tx.query_row(
                "SELECT karus_x,karus_z FROM world_town_starts WHERE zone=?1", [zone], |r| Ok((r.get(0)?,r.get(1)?)))
                .map_err(mlua::Error::external)?;
            let (x,z) = if zone==82 {(202,199)} else {(x,z)};
            st.travel=Some(crate::movement::Position { zone,
                x:x.checked_mul(10).ok_or_else(||mlua::Error::runtime("invalid suppression X"))?,
                z:z.checked_mul(10).ok_or_else(||mlua::Error::runtime("invalid suppression Z"))?, y:0 });
            Ok(())
        });
        function!("HowmuchItem", |_, a: Variadic<i64>| Ok(state
            .borrow()
            .count(*a.get(1).unwrap_or(&0) as u32)));
        function!("GetQuestStatus", |_, a: Variadic<i64>| Ok(*state
            .borrow()
            .quests
            .get(&(*a.get(1).unwrap_or(&0) as u16))
            .unwrap_or(&0)));
        function!("CheckGiveSlot", |_, a: Variadic<i64>| Ok(state
            .borrow()
            .bag
            .iter()
            .filter(|i| i.id == 0)
            .count()
            >= *a.get(1).unwrap_or(&1) as usize));
        function!("SearchQuest", |_, a: Variadic<i64>| {
            let npc = *a.get(1).unwrap_or(&0) as u16;
            let st = state.borrow();
            Ok(
                if all.iter().any(|h| {
                    h.npc == npc
                        && h.flag > 0
                        && h.status < 5 && h.status != 2
                        && h.level <= s.progress.level
                        && eligible(h, s)
                        && open_state(h.status, *st.quests.get(&h.flag).unwrap_or(&0))
                }) {
                    2
                } else {
                    0
                },
            )
        });
        function!("GetPremium", |_, _: Variadic<i64>| Ok(0));
        function!("GetClanGrade", |_, _: Variadic<i64>| Ok(0));
        function!("GetClanRank", |_, _: Variadic<i64>| Ok(0));
        function!("isInClan", |_, _: Variadic<i64>| Ok(false));
        function!("isClanLeader", |_, _: Variadic<i64>| Ok(false));
        function!("GetLoyalty", |_, _: Variadic<i64>| Ok(state.borrow().rewards.totals.loyalty));
        function!("CheckMonsterChallengeTime", |_, _: Variadic<i64>| Ok(-1));
        function!("SelectMsg", |_, a: Variadic<i64>| {
            if a.len() < 5 {
                return Err(mlua::Error::runtime("invalid menu"));
            }
            let mut st = state.borrow_mut();
            let mut p = vec![0x55];
            p.extend(merchant.prototype.to_le_bytes());
            p.push(a[1] as u8);
            let flag=all.iter().find(|h|h.id==st.context.helper).map_or(0,|h|h.flag);
            p.extend((if a[2]>=0 && flag>0 {i32::from(flag)} else {a[2] as i32}).to_le_bytes());
            // Asan's later dialog959 is absent from the original1453 table.
            let talk=if bifrost_script && st.context.file=="16094_Asan.lua" && a[3]==959 {4071} else {a[3] as i32};
            p.extend(talk.to_le_bytes());
            for i in 0..10 {
                p.extend((*a.get(5 + i * 2).unwrap_or(&-1) as i32).to_le_bytes());
                st.context.choices[i] = *a.get(6 + i * 2).unwrap_or(&-1) as i32;
            }
            // Native dialogs read twelve menu slots before the script filename.
            for _ in 0..2 {
                p.extend((-1i32).to_le_bytes());
            }
            p.push(st.context.file.len() as u8);
            p.extend(st.context.file.as_bytes());
            st.packets.push(p);
            Ok(())
        });
        function!("NpcMsg", |_, a: Variadic<i64>| {
            if a.len() < 3 {
                return Err(mlua::Error::runtime("invalid NPC message"));
            }
            let mut p = vec![0x64, 7];
            p.extend((a[1] as u32).to_le_bytes());
            p.extend(merchant.prototype.to_le_bytes());
            state.borrow_mut().packets.push(p);
            Ok(())
        });
        function!("ShowMap", |_, a: Variadic<i64>| {
            let mut p = vec![0x64, 11];
            p.extend((*a.get(1).unwrap_or(&0) as u32).to_le_bytes());
            state.borrow_mut().packets.push(p);
            Ok(())
        });
        function!("SaveEvent", |_, a: Variadic<i64>| {
            let id = *a.get(1).unwrap_or(&0) as u32;
            let h = all
                .iter()
                .find(|h| h.id == id)
                .ok_or_else(|| mlua::Error::runtime("quest helper absent from1453"))?;
            if h.flag == 0 || h.status > 4 {
                return Err(mlua::Error::runtime("unsupported quest state"));
            }
            let mut st = state.borrow_mut();
            let current = *st.quests.get(&h.flag).unwrap_or(&0);
            if current == 2 && h.status != 2 {
                return Err(mlua::Error::runtime("quest already completed"));
            }
            let monster:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM world_reference_rules WHERE table_name='QUEST_MONSTER' AND rule_id=?1)",[h.flag],|r|r.get(0)).map_err(mlua::Error::external)?;
            if matches!(h.status,1|3) {
                crate::quests::check_capacity(&st.quests,h.flag).map_err(mlua::Error::external)?;
            }
            if monster && ((h.status==1 && !matches!(current,1|3)) || matches!(h.status,2|4)) {
                crate::quests::reset_counts(&tx,account,s.character.slot,h.flag).map_err(mlua::Error::external)?;
            }
            st.quests.insert(h.flag, h.status);
            let mut p = vec![0x64, 2];
            p.extend(h.flag.to_le_bytes());
            p.push(h.status);
            st.packets.push(p);
            Ok(())
        });
        lua.globals().set("UpdateEvent", lua.globals().get::<mlua::Function>("SaveEvent")?)?;
        function!("ExistMonsterQuestSub", |_, _: Variadic<i64>| {
            let st=state.borrow();
            let flag=all.iter().find(|h|h.id==st.context.helper).map_or(0,|h|h.flag);
            Ok(if st.quests.get(&flag).is_some_and(|status|matches!(status,1|3)) {flag} else {0})
        });
        function!("CountMonsterQuestSub", |_, a: Variadic<i64>| {
            let st=state.borrow();
            let flag=if a.len()>=3 {*a.get(1).unwrap_or(&0) as u16} else {all.iter().find(|h|h.id==st.context.helper).map_or(0,|h|h.flag)};
            let group=if a.len()>=3 {*a.get(2).unwrap_or(&0) as u16} else {(*a.get(1).unwrap_or(&0) as u16).saturating_sub(32000)};
            if !(1..=4).contains(&group) {return Ok(0);}
            let counts=crate::quests::counts(&tx,account,s.character.slot,flag).map_err(mlua::Error::external)?;
            Ok(counts[usize::from(group-1)])
        });
        function!("GiveItem", |_, a: Variadic<i64>| {
            let c = lobby
                .combat
                .as_ref()
                .ok_or_else(|| mlua::Error::runtime("item catalog unavailable"))?;
            state
                .borrow_mut()
                .give(
                    c,
                    *a.get(1).unwrap_or(&0) as u32,
                    *a.get(2).unwrap_or(&1) as u32,
                )
                .map_err(mlua::Error::external)
        });
        function!("RobItem", |_, a: Variadic<i64>| {
            state
                .borrow_mut()
                .take(
                    *a.get(1).unwrap_or(&0) as u32,
                    *a.get(2).unwrap_or(&1) as u32,
                )
                .map_err(mlua::Error::external)?;
            Ok(true)
        });
        function!("GoldLose", |_, a: Variadic<i64>| {
            state
                .borrow_mut()
                .take(900000000, *a.get(1).unwrap_or(&0) as u32)
                .map_err(mlua::Error::external)?;
            Ok(true)
        });
        function!("GoldGain", |_, a: Variadic<i64>| {
            let c = lobby
                .combat
                .as_ref()
                .ok_or_else(|| mlua::Error::runtime("item catalog unavailable"))?;
            state
                .borrow_mut()
                .give(c, 900000000, *a.get(1).unwrap_or(&0) as u32)
                .map_err(mlua::Error::external)
        });
        function!("CheckExchange", |_, a: Variadic<i64>| {
            let Some(c) = lobby.combat.as_ref() else {
                return Ok(false);
            };
            Ok(exchange(
                &tx,
                (account,s.character.slot),
                c,
                &state.borrow(),
                *a.get(1).unwrap_or(&0) as u32,
                true,
                None,
            )
            .is_ok())
        });
        function!("RunExchange", |_, a: Variadic<i64>| {
            let c = lobby
                .combat
                .as_ref()
                .ok_or_else(|| mlua::Error::runtime("item catalog unavailable"))?;
            let (bag, coins,rewards) = exchange(
                &tx,
                (account,s.character.slot),
                c,
                &state.borrow(),
                *a.get(1).unwrap_or(&0) as u32,
                false,
                None,
            )
            .map_err(mlua::Error::external)?;
            let mut st = state.borrow_mut();
            st.bag = bag;
            st.coins = coins;
            st.rewards=rewards;
            Ok(true)
        });
        function!("CheckQuestExchange", |_,a:Variadic<i64>| {
            let Some(c)=lobby.combat.as_ref() else {return Ok(false);};
            Ok(exchange(&tx,(account,s.character.slot),c,&state.borrow(),*a.get(1).unwrap_or(&0) as u32,true,Some(-1)).is_ok())
        });
        function!("RunQuestExchange", |_,a:Variadic<i64>| {
            let c=lobby.combat.as_ref().ok_or_else(||mlua::Error::runtime("item catalog unavailable"))?;
            let (bag,coins,rewards)=exchange(&tx,(account,s.character.slot),c,&state.borrow(),*a.get(1).unwrap_or(&0) as u32,false,Some(action.1.max(0))).map_err(mlua::Error::external)?;
            let mut st=state.borrow_mut();st.bag=bag;st.coins=coins;st.rewards=rewards;Ok(true)
        });
        let filename = state.borrow().context.file.clone();
        lua.load(script).set_name(filename).exec()
    })?;
    let mut result = state.into_inner();
    anyhow::ensure!(
        menu_reply
            || !result.packets.is_empty()
            || result.bag != before.0
            || result.coins != before.1
            || result.quests != before.2
            || result.rewards != before.3
            || result.class.is_some()
            || result.travel.is_some(),
        "no matching event in supplied NPC script"
    );
    let c = lobby
        .combat
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("item catalog unavailable"))?;
    let equipped = c.calculate(&s.character, &s.progress, &s.equipment)?.weight;
    anyhow::ensure!(
        u32::from(equipped) + u32::from(c.bag_weight(&result.bag))
            <= u32::from(s.combat.max_weight),
        "inventory too heavy"
    );
    for (i, item) in result.bag.iter().enumerate() {
        tx.execute(
            "DELETE FROM character_inventory WHERE account=?1 AND slot=?2 AND inventory_slot=?3",
            params![account, s.character.slot, i],
        )?;
        if item.id != 0 {
            tx.execute(
                "INSERT INTO character_inventory VALUES (?1,?2,?3,?4,?5,?6)",
                params![
                    account,
                    s.character.slot,
                    i,
                    item.id,
                    item.durability,
                    item.count
                ],
            )?;
        }
    }
    tx.execute(
        "UPDATE character_wallets SET coins=?3 WHERE account=?1 AND slot=?2",
        params![account, s.character.slot, result.coins],
    )?;
    tx.execute(
        "DELETE FROM character_quests WHERE account=?1 AND slot=?2",
        params![account, s.character.slot],
    )?;
    for (quest, status) in &result.quests {
        tx.execute("INSERT INTO character_quests VALUES (?1,?2,?3,?4) ON CONFLICT(account,slot,quest) DO UPDATE SET state=excluded.state",params![account,s.character.slot,quest,status])?;
    }
    if let Some(position) = result.travel {
        crate::movement::save(&tx, account, s.character.slot, position)?;
    }
    if result.rewards != before.3 {
        result.rewards.save(&tx, account, s.character.slot)?;
    }
    if let Some(class) = result.class {
        anyhow::ensure!(
            tx.execute(
                "UPDATE lobby_characters SET class=?3 WHERE account=?1 AND slot=?2 AND class=?4",
                params![account, s.character.slot, class, s.character.class]
            )? == 1,
            "character class changed concurrently"
        );
    }
    if result.rewards.progress.level != s.progress.level || result.class.is_some() {
        let mut next = s.clone();
        if let Some(class) = result.class {
            next.character.class = class;
        }
        next.progress = result.rewards.progress.clone();
        next.inventory = result.bag;
        crate::durability::refresh(&mut next, c, lobby.skills.as_ref());
        next.skills.set_limits(next.combat.hp, next.combat.mp);
        if next.progress.level != s.progress.level {
            next.skills.hp = next.skills.max_hp;
            next.skills.mana = next.skills.max_mana;
        }
        crate::skills::save_in(&tx, account, s.character.slot, &next.skills)?;
        result.level_state = Some((next.base_combat, next.combat, next.skills));
    }
    tx.commit()?;
    if let Some(hub) = abyss_hub.as_mut() {
        for (id, changed) in party_items.into_inner() {
            hub.refresh.insert(id);
            hub.send(id, inventory::count_packet(&changed));
        }
        if let Some(destination) = party_travel.into_inner() {
            for player in party {
                hub.transfers.insert(player.s.player_id, destination);
            }
        }
    }
    Ok(result)
}

pub(crate) fn cancellation_request(
    db: &Connection,
    account: &str,
    s: &Session,
    flag: u16,
) -> anyhow::Result<Vec<u8>> {
    anyhow::ensure!(flag > 0 && flag < 32000, "Invalid quest ID.");
    let active:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM character_quests WHERE account=?1 AND slot=?2 AND quest=?3 AND state IN (1,3))",params![account,s.character.slot,flag],|r|r.get(0))?;
    anyhow::ensure!(active, "This quest is not active on your character.");
    let helper = helpers(db)?
        .into_iter()
        .find(|h| h.flag == flag && h.kind == 1 && eligible(h, s))
        .ok_or_else(|| {
            anyhow::anyhow!("This quest cannot be abandoned through the quest journal.")
        })?;
    let mut p = vec![0x64, 5];
    p.extend(helper.id.to_le_bytes());
    Ok(p)
}

pub(crate) async fn handle(
    sock: &mut TcpStream,
    peer: SocketAddr,
    lobby: &Lobby,
    account: &str,
    s: &mut Session,
    p: &[u8],
) {
    if crate::clan_promotion::handle(sock, peer, lobby, account, s, p).await {
        return;
    }
    let progress_reset=s.entered && p.len()==6 && p[..2]==[0x64,12] && (||->anyhow::Result<bool> {
        let db=lobby.db.lock().unwrap();
        let id=u32::from_le_bytes(p[2..6].try_into()?);
        let Some(h)=helpers(&db)?.into_iter().find(|h|h.id==id) else {return Ok(false);};
        Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM character_quests WHERE account=?1 AND slot=?2 AND quest=?3 AND state IN (1,3))",params![account,s.character.slot,h.flag],|r|r.get(0))?)
    })().unwrap_or(false);
    if s.entered && p.len() == 6 && (p[..2] == [0x64, 4] || progress_reset) {
        match check_progress(lobby, account, s, p) {
            Ok(packets) => {
                for packet in packets {
                    super::send_response(sock, peer, &None, &packet, "QUEST_OBJECTIVE_CHECK").await;
                }
            }
            Err(error) => tracing::warn!(%peer,%error,"quest progress check refused"),
        }
        return;
    }
    if s.entered && p.len() == 6 && p[..2] == [0x64, 5] {
        let id = u32::from_le_bytes(p[2..6].try_into().unwrap());
        let result = (|| -> anyhow::Result<(u16, Option<crate::movement::Position>)> {
            let db = lobby.db.lock().unwrap();
            let h = helpers(&db)?
                .into_iter()
                .find(|h| h.id == id && h.flag > 0 && eligible(h, s))
                .ok_or_else(|| anyhow::anyhow!("invalid quest cancellation"))?;
            let destination = if matches!((h.flag, s.position.zone), (603, 82) | (605, 83)) {
                let nation = s.character.nation();
                let (kx, kz, ex, ez): (u16, u16, u16, u16) = db.query_row(
                    "SELECT karus_x,karus_z,elmo_x,elmo_z FROM world_town_starts WHERE zone=?1",
                    [nation],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )?;
                let (x, z) = if nation == 1 { (kx, kz) } else { (ex, ez) };
                Some(crate::movement::Position {
                    zone: nation,
                    x: x.checked_mul(10)
                        .ok_or_else(|| anyhow::anyhow!("invalid home X"))?,
                    z: z.checked_mul(10)
                        .ok_or_else(|| anyhow::anyhow!("invalid home Z"))?,
                    y: 0,
                })
            } else {
                None
            };
            if destination.is_some() {
                crate::quests::cancel_at(&db, account, s.character.slot, h.flag, destination)?;
            } else {
                crate::quests::cancel(&db, account, s.character.slot, h.flag)?;
            }
            Ok((h.flag, destination))
        })();
        match result {
            Ok((flag, destination)) => {
                s.npc_dialog = None;
                let mut packet = vec![0x64, 2];
                packet.extend(flag.to_le_bytes());
                packet.push(4);
                super::send_response(sock, peer, &None, &packet, "QUEST_CANCELLED").await;
                let counts = crate::quests::monster_snapshot(
                    &lobby.db.lock().unwrap(),
                    account,
                    s.character.slot,
                );
                if let Ok(counts) = counts {
                    super::send_response(sock, peer, &None, &counts, "QUEST_COUNTS").await;
                }
                if let Some(position) = destination {
                    let packet = crate::travel::begin(lobby, s, position);
                    super::send_response(sock, peer, &None, &packet, "SUPPRESSION_CANCEL_RETURN")
                        .await;
                }
            }
            Err(error) => tracing::info!(%peer,%error,"quest cancellation refused"),
        }
        return;
    }
    let result = (|| -> anyhow::Result<Changes> {
        anyhow::ensure!(s.entered && s.skills.hp > 0, "character unavailable");
        let all = helpers(&lobby.db.lock().unwrap())?;
        let (context, event, step) = match p.first() {
            Some(0x20) if matches!(p.len(), 3 | 7) => {
                let id = u16::from_le_bytes([p[1], p[2]]);
                let merchant = lobby
                    .merchants
                    .iter()
                    .find(|m| m.id == id && m.nearby(s))
                    .ok_or_else(|| anyhow::anyhow!("NPC out of range"))?;
                let requested = if p.len() == 7 {
                    u32::from_le_bytes(p[3..7].try_into()?)
                } else {
                    0
                };
                let h = all
                    .iter()
                    .find(|h| {
                        h.npc == merchant.prototype
                            && eligible(h, s)
                            && if requested == 0 {
                                h.flag == 0 && h.status == 0
                            } else {
                                h.id == requested && h.level <= s.progress.level
                            }
                    })
                    .ok_or_else(|| anyhow::anyhow!("no compatible NPC event"))?;
                let h = entry_helper(&all, h);
                if requested != 0 && h.flag != 0 {
                    let state=lobby.db.lock().unwrap().query_row("SELECT COALESCE((SELECT state FROM character_quests WHERE account=?1 AND slot=?2 AND quest=?3),0)",params![account,s.character.slot,h.flag],|r|r.get::<_,u8>(0))?;
                    anyhow::ensure!(open_state(h.status, state), "quest state mismatch");
                }
                (
                    Context {
                        npc: id,
                        helper: h.id,
                        file: h.file.clone(),
                        choices: [-1; 10],
                    },
                    h.event,
                    -1,
                )
            }
            Some(0x55) if p.len() >= 2 => {
                let context = s
                    .npc_dialog
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("no active NPC dialog"))?;
                let index = usize::from(p[1] % 10);
                anyhow::ensure!(
                    index < 10 && context.choices[index] >= 0,
                    "invalid menu choice"
                );
                if p[1] >= 10 {
                    let h = all
                        .iter()
                        .find(|h| h.id == context.helper)
                        .ok_or_else(|| anyhow::anyhow!("invalid quest helper"))?;
                    let text:String=lobby.db.lock().unwrap().query_row("SELECT data FROM world_reference_rules WHERE table_name='ITEM_EXCHANGE' AND rule_id=?1",[h.exchange],|r|r.get(0))?;
                    let row: serde_json::Value = serde_json::from_str(&text)?;
                    anyhow::ensure!(
                        number(&row, "bRandomFlag") == 12
                            && p[1] / 10 < 5
                            && number(&row, &format!("nExchangeItemNum{}", p[1] / 10 + 1)) != 0,
                        "invalid packed quest reward"
                    );
                }
                if p.len() > 2 {
                    let n = usize::from(p[2]);
                    anyhow::ensure!(
                        matches!(p.len(), length if length==3+n || length==4+n)
                            && (n == 0 || p.get(3..3 + n) == Some(context.file.as_bytes())),
                        "script filename mismatch"
                    );
                }
                let event = context.choices[index];
                let step = if p.len() > 2 && p.len() == 4 + usize::from(p[2]) {
                    *p.last().unwrap() as i8
                } else {
                    if p[1] >= 10 { (p[1] / 10) as i8 } else { -1 }
                };
                (context, event, step)
            }
            Some(0x64) if p.len() == 6 && matches!(p[1], 3 | 4 | 6 | 7 | 12) => {
                let id = u32::from_le_bytes(p[2..6].try_into()?);
                let h = all
                    .iter()
                    .find(|h| h.id == id && h.level <= s.progress.level && eligible(h, s))
                    .ok_or_else(|| anyhow::anyhow!("invalid quest helper"))?;
                let h = entry_helper(&all, h);
                let merchant = lobby
                    .merchants
                    .iter()
                    .find(|m| m.prototype == h.npc && m.nearby(s))
                    .ok_or_else(|| anyhow::anyhow!("quest NPC out of range"))?;
                let accepting = matches!(p[1], 6 | 12);
                if accepting {
                    anyhow::ensure!(
                        s.npc_dialog
                            .as_ref()
                            .is_some_and(|context| context.helper == h.id
                                && context.choices.contains(&2147483001)),
                        "quest was not offered"
                    );
                }
                let state=lobby.db.lock().unwrap().query_row("SELECT COALESCE((SELECT state FROM character_quests WHERE account=?1 AND slot=?2 AND quest=?3),0)",params![account,s.character.slot,h.flag],|r|r.get::<_,u8>(0))?;
                anyhow::ensure!(
                    h.flag == 0
                        || if p[1] == 4 {
                            matches!(state, 1 | 3) && h.complete > 0
                        } else {
                            open_state(h.status, state)
                        },
                    "quest state mismatch"
                );
                (
                    Context {
                        npc: merchant.id,
                        helper: h.id,
                        file: h.file.clone(),
                        choices: [-1; 10],
                    },
                    if accepting {
                        2147483001
                    } else if p[1] == 4 {
                        h.complete
                    } else {
                        h.event
                    },
                    -1,
                )
            }
            _ => anyhow::bail!("unsupported NPC request"),
        };
        let merchant = lobby
            .merchants
            .iter()
            .find(|m| m.id == context.npc && m.nearby(s))
            .ok_or_else(|| anyhow::anyhow!("NPC out of range"))?;
        anyhow::ensure!(
            context.file.ends_with(".lua")
                && context
                    .file
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.')),
            "invalid script path"
        );
        let path = reko_common::game_dir()
            .parent()
            .ok_or_else(|| anyhow::anyhow!("game directory unavailable"))?
            .join("kn_2086_patch_private_server/lCAFILES/Quests")
            .join(&context.file);
        let script = if let Some(script) = crate::hemes::script(&context.file) {
            anyhow::ensure!(
                merchant.prototype == 16085 && merchant.kind == 132 && s.position.zone == 21,
                "Peddler service unavailable"
            );
            script
        } else if let Some(script) = database_quest_script(&all, &context) {
            script
        } else {
            std::fs::read(path)?
        };
        execute(lobby, account, s, merchant, context, (event, step), &script)
    })();
    match result {
        Ok(result) => {
            let item_update = inventory::reward_packet(&s.inventory, &result.bag);
            tracing::info!(%peer,helper=result.context.helper,npc=result.context.npc,"NPC script committed");
            let old = s.coins;
            let old_rewards = s.rewards;
            let old_level = s.progress.level;
            s.inventory = result.bag;
            s.coins = result.coins;
            s.rewards = result.rewards.totals;
            s.progress = result.rewards.progress;
            s.stat_points = result.rewards.stat_points;
            if let Some(class) = result.class {
                s.character.class = class;
                let mut packet = vec![0x34, 6];
                packet.extend(class.to_le_bytes());
                packet.extend(s.player_id.to_le_bytes());
                super::send_response(sock, peer, &None, &packet, "FIRST_JOB_CHANGE").await;
            }
            if result.rewards.max_exp > 0 {
                s.max_exp = result.rewards.max_exp;
            }
            if let Some((base, stats, skills)) = result.level_state {
                s.base_combat = base;
                s.combat = stats;
                s.skills = skills;
            }
            if result.class.is_some() {
                super::send_response(
                    sock,
                    peer,
                    &None,
                    &inventory::attributes(&s.combat, 2),
                    "JOB_ATTRIBUTES",
                )
                .await;
                super::send_response(sock, peer, &None, &s.skills.hp_packet(0), "JOB_HP").await;
                super::send_response(sock, peer, &None, &s.skills.mana_packet(), "JOB_MP").await;
            }
            s.npc_dialog = Some(result.context);
            for p in result.packets {
                super::send_response(sock, peer, &None, &p, "NPC_SCRIPT").await;
            }
            if s.progress.level != old_level {
                super::send_response(
                    sock,
                    peer,
                    &None,
                    &crate::quest_rewards::level_packet(s),
                    "QUEST_LEVEL_REWARD",
                )
                .await;
                super::send_response(
                    sock,
                    peer,
                    &None,
                    &inventory::attributes(&s.combat, 2),
                    "QUEST_LEVEL_ATTRIBUTES",
                )
                .await;
            } else if old_rewards.experience != s.rewards.experience {
                super::send_response(
                    sock,
                    peer,
                    &None,
                    &crate::quest_rewards::experience_packet(s),
                    "QUEST_EXPERIENCE_REWARD",
                )
                .await;
            }
            if old_rewards.loyalty != s.rewards.loyalty {
                super::send_response(
                    sock,
                    peer,
                    &None,
                    &crate::quest_rewards::loyalty_packet(s),
                    "QUEST_NATIONAL_POINT_REWARD",
                )
                .await;
            }
            let counts = {
                let db = lobby.db.lock().unwrap();
                let preferred = s
                    .npc_dialog
                    .as_ref()
                    .and_then(|context| {
                        helpers(&db)
                            .ok()?
                            .into_iter()
                            .find(|h| h.id == context.helper)
                            .map(|h| h.flag)
                    })
                    .unwrap_or(0);
                crate::quests::monster_snapshot_for(&db, account, s.character.slot, preferred)
            };
            if let Ok(counts) = counts {
                super::send_response(sock, peer, &None, &counts, "QUEST_COUNTS").await;
            }
            if old != s.coins {
                let p = crate::wallet::change(old, s.coins);
                super::send_response(sock, peer, &None, &p, "NPC_SCRIPT_COINS").await;
            }
            if let Some(item_update) = item_update {
                super::send_response(sock, peer, &None, &item_update, "NPC_SCRIPT_ITEMS").await;
                if let Some(catalog) = lobby.combat.as_ref() {
                    crate::durability::refresh(s, catalog, lobby.skills.as_ref());
                    let mut weight = vec![0x54];
                    weight.extend(s.combat.weight.to_le_bytes());
                    super::send_response(sock, peer, &None, &weight, "QUEST_ITEM_WEIGHT").await;
                }
            }
            if let Some(c) = lobby.combat.as_ref() {
                crate::durability::refresh(s, c, lobby.skills.as_ref());
            }
            if let Some(position) = result.travel {
                let packet = crate::travel::begin(lobby, s, position);
                super::send_response(sock, peer, &None, &packet, "SUPPRESSION_ZONE_CHANGE").await;
                if s.entered {
                    crate::population::refresh(sock, peer, lobby, s).await;
                }
            }
        }
        Err(error) => {
            tracing::warn!(%peer,%error,"NPC script request refused atomically");
            let message = if error.to_string().contains("quest log is full") {
                "Your quest log is full (40 active quests). Complete or abandon a quest first."
            } else {
                "This NPC action is unavailable in the supplied 1453 content. No items or coins were taken."
            };
            let p = crate::chat::packet(7, s.character.nation(), u16::MAX, "", message.as_bytes());
            super::send_response(sock, peer, &None, &p, "NPC_SCRIPT_UNAVAILABLE").await;
        }
    }
}

#[cfg(test)]
#[path = "abyss_tests.rs"]
mod abyss_tests;

#[cfg(test)]
#[path = "bifrost_tests.rs"]
mod bifrost_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn suppression_turn_in_rebuilds_reused_icons_and_reports_both_source_rewards_once() {
        let Some((mut lobby, mut s)) = setup() else {
            return;
        };
        s.position.zone = 1;
        lobby.merchants[0].prototype = 21510;
        lobby.merchants[0].position = s.position;
        {
            let db = lobby.db.lock().unwrap();
            for (id, status) in [(9011, 1), (9012, 2), (9013, 3)] {
                let row = serde_json::json!({"sNpcId":21510,"sEventDataIndex":603,"bEventStatus":status,"bLevel":46,"bClass":2,"bNation":1,"bQuestType":1,"nExchangeIndex":95,"nEventTalkIndex":6071,"nEventTriggerIndex":6099,"strLuaFilename":"21510_Hwargo.lua"});
                db.execute(
                    "INSERT INTO world_reference_rules VALUES('QUEST_HELPER',?1,?2)",
                    params![id, row.to_string()],
                )
                .unwrap();
            }
            let row = serde_json::json!({"bRandomFlag":11,"nOriginItemNum1":910138000,"nOriginItemCount1":3,"nOriginItemNum2":910135000,"nOriginItemCount2":1,"nExchangeItemNum1":168110045,"nExchangeItemCount1":1,"nExchangeItemNum2":111010045,"nExchangeItemCount2":1});
            db.execute(
                "INSERT INTO world_reference_rules VALUES('ITEM_EXCHANGE',95,?1)",
                [row.to_string()],
            )
            .unwrap();
            db.execute("INSERT INTO character_quests VALUES('test',0,603,1)", [])
                .unwrap();
            for (slot, id, count) in [(0, 910138000, 3), (1, 910135000, 1), (2, 391010000, 99)] {
                db.execute(
                    "INSERT INTO character_inventory VALUES('test',0,?1,?2,1,?3)",
                    params![slot, id, count],
                )
                .unwrap();
            }
            s.inventory = starter::inventory(&db, "test", 0).unwrap();
        }
        let original = s.inventory;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (mut socket, peer) = listener.accept().await.unwrap();
        let mut request = vec![0x64, 7];
        request.extend(9013u32.to_le_bytes());
        handle(&mut socket, peer, &lobby, "test", &mut s, &request).await;
        receive_until(&mut client, 0x55).await;
        handle(&mut socket, peer, &lobby, "test", &mut s, &[0x55, 0, 0]).await;
        let replies = receive_until(&mut client, 0x54).await;
        let update = replies.iter().find(|p| p[0] == 0x3d).unwrap();
        assert_eq!(u16::from_le_bytes(update[1..3].try_into().unwrap()), 4);
        // Model the native replacement branch: replacing a nonempty slot leaves
        // its new icon null, which crashes the next merchant inventory transfer.
        let mut icons = original.map(|item| (item.id, item.id != 0));
        let mut notices = Vec::new();
        let (entries, remainder) = update[3..].as_chunks::<21>();
        assert!(remainder.is_empty());
        for entry in entries {
            let index = usize::from(entry[1]);
            let id = u32::from_le_bytes(entry[2..6].try_into().unwrap());
            let count = u32::from_le_bytes(entry[6..10].try_into().unwrap());
            assert_eq!(entry[0], 1);
            if count == 0 {
                icons[index] = (0, false);
            } else if icons[index].0 == 0 {
                icons[index] = (id, true);
            } else if icons[index].0 != id {
                icons[index] = (id, false);
            }
            if entry[10] == 100 {
                notices.push(id);
            }
        }
        assert_eq!(notices, [168110045, 111010045]);
        assert_eq!(
            &icons[..3],
            &[(168110045, true), (111010045, true), (391010000, true)]
        );
        assert_eq!(s.inventory[2], original[2]);
        let saved = s.inventory;
        {
            let db = lobby.db.lock().unwrap();
            assert_eq!(starter::inventory(&db, "test", 0).unwrap(), saved);
            assert_eq!(db.query_row("SELECT state FROM character_quests WHERE account='test' AND slot=0 AND quest=603",[],|row|row.get::<_,u8>(0)).unwrap(),2);
        }
        handle(&mut socket, peer, &lobby, "test", &mut s, &[0x55, 0, 0]).await;
        assert_eq!(s.inventory, saved);
        assert_eq!(s.coins, 1000000);
    }
    #[tokio::test]
    async fn suppression_solo_entry_commits_quest_and_position_together() {
        for (flag, zone, level) in [(603u16, 82u8, 46u8), (605, 83, 64)] {
            let Some((mut lobby, mut s)) = setup() else {
                return;
            };
            s.position.zone = 1;
            lobby.merchants[0].prototype = 21510;
            lobby.merchants[0].position = s.position;
            let npc = lobby.merchants[0].id;
            {
                let db = lobby.db.lock().unwrap();
                db.execute(
                    "INSERT INTO world_town_starts VALUES (?1,200,200,200,200,0,0,'test/source')",
                    [zone],
                )
                .unwrap();
                for (class, base) in [(1, 9000), (2, 9010)] {
                    db.execute("INSERT OR IGNORE INTO world_town_starts VALUES (1,100,110,100,110,0,0,'test/home')",[]).unwrap();
                    for (offset, status) in [(0, 0), (1, 1), (2, 2), (4, 255)] {
                        let row = serde_json::json!({"sNpcId":21510,"sEventDataIndex":flag,"bEventStatus":status,"bLevel":level,"bClass":class,"bNation":1,"bQuestType":1,"nExchangeIndex":95,"nEventTalkIndex":6071,"nEventTriggerIndex":6092,"strLuaFilename":"21510_Hwargo.lua"});
                        db.execute(
                            "INSERT INTO world_reference_rules VALUES('QUEST_HELPER',?1,?2)",
                            params![base + offset, row.to_string()],
                        )
                        .unwrap();
                    }
                }
                db.execute(
                    "INSERT INTO world_reference_rules VALUES('ITEM_EXCHANGE',95,?1)",
                    [r#"{"nOriginItemNum1":910135000,"nOriginItemCount1":1}"#],
                )
                .unwrap();
                db.execute_batch("CREATE TRIGGER fail_suppression BEFORE INSERT ON character_positions BEGIN SELECT RAISE(ABORT,'test position failure'); END;").unwrap();
            }
            let all = helpers(&lobby.db.lock().unwrap()).unwrap();
            let context = Context {
                npc,
                helper: 9010,
                file: "21510_Hwargo.lua".into(),
                choices: [2147483001; 10],
            };
            let script = database_quest_script(&all, &context).unwrap();
            assert!(String::from_utf8_lossy(&script).contains("SaveEvent(UID,9011)"));
            assert!(
                execute(
                    &lobby,
                    "test",
                    &s,
                    &lobby.merchants[0],
                    context,
                    (2147483001, -1),
                    &script
                )
                .is_err()
            );
            {
                let db = lobby.db.lock().unwrap();
                assert_eq!(
                    db.query_row(
                        "SELECT count(*) FROM character_quests WHERE account='test' AND quest=?1",
                        [flag],
                        |r| r.get::<_, u32>(0)
                    )
                    .unwrap(),
                    0
                );
                db.execute_batch("DROP TRIGGER fail_suppression").unwrap();
            }
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let mut client = TcpStream::connect(listener.local_addr().unwrap())
                .await
                .unwrap();
            let (mut socket, peer) = listener.accept().await.unwrap();
            let mut request = vec![0x64, 7];
            request.extend(9014u32.to_le_bytes());
            handle(&mut socket, peer, &lobby, "test", &mut s, &request).await;
            assert_eq!(s.npc_dialog.as_ref().unwrap().helper, 9010);
            receive_until(&mut client, 0x55).await;
            handle(&mut socket, peer, &lobby, "test", &mut s, &[0x55, 0, 0]).await;
            let replies = receive_until(&mut client, 0x27).await;
            assert_eq!(&replies.last().unwrap()[..4], &[0x27, 3, zone, 0]);
            assert!(s.zone_loading && !s.entered && s.npc_dialog.is_none());
            assert_eq!(s.position.zone, zone);
            {
                let db = lobby.db.lock().unwrap();
                assert_eq!(crate::movement::load(&db, "test", 0).unwrap(), s.position);
                assert_eq!(
                    db.query_row(
                        "SELECT state FROM character_quests WHERE account='test' AND quest=?1",
                        [flag],
                        |r| r.get::<_, u8>(0)
                    )
                    .unwrap(),
                    1
                );
                assert_eq!(starter::coins(&db, "test", 0).unwrap(), 1000000);
                assert!(
                    starter::inventory(&db, "test", 0)
                        .unwrap()
                        .iter()
                        .all(|i| i.id == 0)
                );
            }
            s.entered = true;
            s.zone_loading = false;
            let mut cancel = vec![0x64, 5];
            cancel.extend(9010u32.to_le_bytes());
            handle(&mut socket, peer, &lobby, "test", &mut s, &cancel).await;
            let replies = receive_until(&mut client, 0x27).await;
            assert_eq!(&replies.last().unwrap()[..4], &[0x27, 3, 1, 0]);
            assert_eq!(
                (s.position.zone, s.position.x, s.position.z),
                (1, 1000, 1100)
            );
            let db = lobby.db.lock().unwrap();
            assert_eq!(
                db.query_row(
                    "SELECT state FROM character_quests WHERE account='test' AND quest=?1",
                    [flag],
                    |r| r.get::<_, u8>(0)
                )
                .unwrap(),
                4
            );
        }
    }
    #[tokio::test]
    async fn forty_quests_accept_and_count_independently_without_erasing_other_progress() {
        let Some((lobby, mut s)) = setup() else {
            return;
        };
        {
            let db = lobby.db.lock().unwrap();
            for index in 0..41u16 {
                let flag = 1000 + index;
                for (offset, status) in [(0, 1), (1, 2), (2, 4)] {
                    let row = serde_json::json!({"sNpcId":"16079","sEventDataIndex":flag,"bEventStatus":status,"bLevel":1,"bClass":5,"bNation":3,"bQuestType":1,"nExchangeIndex":6000+index,"nEventTalkIndex":176,"nEventTriggerIndex":175,"strLuaFilename":"16079_Menisia.lua"});
                    db.execute(
                        "INSERT INTO world_reference_rules VALUES('QUEST_HELPER',?1,?2)",
                        params![10000 + index * 3 + offset, row.to_string()],
                    )
                    .unwrap();
                }
                let row =
                    serde_json::json!({"sNum1a":"700","sCount1":if index==1 {"3"} else {"2"}});
                db.execute(
                    "INSERT INTO world_reference_rules VALUES('QUEST_MONSTER',?1,?2)",
                    params![flag, row.to_string()],
                )
                .unwrap();
                let row = serde_json::json!({"bRandomFlag":"10","nOriginItemNum1":"900005000","nOriginItemCount1":flag,"nExchangeItemNum1":"900001000","nExchangeItemCount1":"50"});
                db.execute(
                    "INSERT INTO world_reference_rules VALUES('ITEM_EXCHANGE',?1,?2)",
                    params![6000 + index, row.to_string()],
                )
                .unwrap();
            }
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let _client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (mut socket, peer) = listener.accept().await.unwrap();
        let request = |id: u32| {
            let mut p = vec![0x64, 7];
            p.extend(id.to_le_bytes());
            p
        };
        for index in 0..41u32 {
            handle(
                &mut socket,
                peer,
                &lobby,
                "test",
                &mut s,
                &request(10002 + index * 3),
            )
            .await;
            handle(&mut socket, peer, &lobby, "test", &mut s, &[0x55, 0, 0]).await;
        }
        {
            let db = lobby.db.lock().unwrap();
            let active:u32=db.query_row("SELECT count(*) FROM character_quests WHERE account='test' AND slot=0 AND state IN (1,3) AND quest<32000",[],|r|r.get(0)).unwrap();
            assert_eq!(active, 40);
            assert!(
                !db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM character_quests WHERE quest=1040)",
                    [],
                    |r| r.get::<_, bool>(0)
                )
                .unwrap()
            );
            for count in 1..=2 {
                let packets = crate::quests::kill(&db, "test", 0, 700).unwrap();
                assert_eq!(packets.len(), 80);
                for index in 0..40u16 {
                    let mut snapshot = vec![0x64, 9, 1];
                    snapshot.extend((1000 + index).to_le_bytes());
                    snapshot.extend([count - 1, 0, 0, 0]);
                    assert_eq!(packets[usize::from(index) * 2], snapshot);
                    assert_eq!(packets[usize::from(index) * 2 + 1], [0x64, 9, 2, 1, count]);
                    assert_eq!(
                        crate::quests::counts(&db, "test", 0, 1000 + index).unwrap(),
                        [count, 0, 0, 0]
                    );
                }
            }
            crate::quests::initialize(&db).unwrap();
            assert_eq!(
                crate::quests::counts(&db, "test", 0, 1001).unwrap(),
                [2, 0, 0, 0]
            );
        }
        assert_eq!(
            check_progress(&lobby, "test", &s, &[0x64, 4, 0x10, 0x27, 0, 0]).unwrap(),
            [vec![0x64, 2, 0xe8, 3, 3]]
        );
        handle(&mut socket, peer, &lobby, "test", &mut s, &request(10000)).await;
        handle(&mut socket, peer, &lobby, "test", &mut s, &[0x55, 0, 0]).await;
        assert_eq!(s.rewards.experience, 50);
        {
            let db = lobby.db.lock().unwrap();
            assert!(crate::quests::completed(&db, "test", 0, 1000).unwrap());
            assert_eq!(crate::quests::counts(&db, "test", 0, 1000).unwrap(), [0; 4]);
            assert_eq!(
                crate::quests::counts(&db, "test", 0, 1001).unwrap(),
                [2, 0, 0, 0]
            );
            assert_eq!(
                crate::quests::kill(&db, "test", 0, 700).unwrap(),
                [
                    vec![0x64, 9, 1, 0xe9, 3, 2, 0, 0, 0],
                    vec![0x64, 9, 2, 1, 3]
                ]
            );
            crate::quests::cancel(&db, "test", 0, 1001).unwrap();
            assert_eq!(
                crate::quests::counts(&db, "test", 0, 1002).unwrap(),
                [2, 0, 0, 0]
            );
        }
        for id in [10005, 10122] {
            handle(&mut socket, peer, &lobby, "test", &mut s, &request(id)).await;
            handle(&mut socket, peer, &lobby, "test", &mut s, &[0x55, 0, 0]).await;
        }
        let db = lobby.db.lock().unwrap();
        assert_eq!(crate::quests::counts(&db, "test", 0, 1001).unwrap(), [0; 4]);
        assert_eq!(
            crate::quests::counts(&db, "test", 0, 1002).unwrap(),
            [2, 0, 0, 0]
        );
        assert_eq!(
            db.query_row(
                "SELECT state FROM character_quests WHERE account='test' AND slot=0 AND quest=1040",
                [],
                |r| r.get::<_, u8>(0)
            )
            .unwrap(),
            1
        );
    }
    async fn receive_until(client: &mut TcpStream, opcode: u8) -> Vec<Vec<u8>> {
        use tokio::io::AsyncReadExt;
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            let mut packets = Vec::new();
            loop {
                let mut header = [0; 4];
                client.read_exact(&mut header).await.unwrap();
                assert_eq!(&header[..2], &[0xaa, 0x55]);
                let len = usize::from(u16::from_le_bytes([header[2], header[3]]));
                let mut body = vec![0; len + 2];
                client.read_exact(&mut body).await.unwrap();
                assert_eq!(&body[len..], &[0x55, 0xaa]);
                body.truncate(len);
                let done = body[0] == opcode;
                packets.push(body);
                if done {
                    return packets;
                }
            }
        })
        .await
        .expect("expected immediate native update packet")
    }
    #[tokio::test]
    async fn item_progress_is_remote_read_only_reversible_and_reward_notifies_wallet_and_xp() {
        let Some((lobby, mut s)) = setup() else {
            return;
        };
        {
            let db = lobby.db.lock().unwrap();
            for (id, status) in [(900, 1), (901, 3), (902, 2)] {
                let row = serde_json::json!({"sNpcId":"16079","sEventDataIndex":"102","bEventStatus":status,"bLevel":1,"bClass":5,"bNation":3,"bQuestType":1,"nExchangeIndex":6,"nEventTalkIndex":201,"nEventTriggerIndex":200,"strLuaFilename":"16079_Menisia.lua"});
                db.execute(
                    "INSERT INTO world_reference_rules VALUES('QUEST_HELPER',?1,?2)",
                    params![id, row.to_string()],
                )
                .unwrap();
            }
            db.execute("INSERT INTO world_reference_rules VALUES('ITEM_EXCHANGE',6,?1)",[r#"{"bRandomFlag":"10","nOriginItemNum1":"391010000","nOriginItemCount1":"2","nExchangeItemNum1":"900000000","nExchangeItemCount1":"50000","nExchangeItemNum2":"900001000","nExchangeItemCount2":"500"}"#]).unwrap();
            db.execute("INSERT INTO character_quests VALUES('test',0,102,1)", [])
                .unwrap();
            db.execute(
                "INSERT INTO character_inventory VALUES('test',0,0,391010000,0,1)",
                [],
            )
            .unwrap();
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (mut socket, peer) = listener.accept().await.unwrap();
        let home = s.position;
        s.position.x = 500;
        s.npc_dialog = None;
        let request = |sub: u8, id: u32| {
            let mut p = vec![0x64, sub];
            p.extend(id.to_le_bytes());
            p
        };
        let status = || {
            lobby.db.lock().unwrap().query_row("SELECT state FROM character_quests WHERE account='test' AND slot=0 AND quest=102",[],|r|r.get::<_,u8>(0)).unwrap()
        };
        handle(&mut socket, peer, &lobby, "test", &mut s, &request(4, 900)).await;
        assert_eq!(status(), 1);
        assert!(s.npc_dialog.is_none());
        lobby
            .db
            .lock()
            .unwrap()
            .execute(
                "UPDATE character_inventory SET count=2 WHERE account='test'",
                [],
            )
            .unwrap();
        handle(&mut socket, peer, &lobby, "test", &mut s, &request(4, 900)).await;
        assert_eq!(
            receive_until(&mut client, 0x64).await,
            [vec![0x64, 2, 102, 0, 3]]
        );
        assert_eq!(status(), 3);
        assert_eq!((s.coins, s.rewards.experience), (1000000, 0));
        assert!(
            check_progress(&lobby, "test", &s, &request(4, 900))
                .unwrap()
                .is_empty(),
            "unchanged progress must not cause a native callback loop"
        );
        lobby
            .db
            .lock()
            .unwrap()
            .execute(
                "UPDATE character_inventory SET count=1 WHERE account='test'",
                [],
            )
            .unwrap();
        handle(&mut socket, peer, &lobby, "test", &mut s, &request(12, 901)).await;
        assert_eq!(
            receive_until(&mut client, 0x64).await,
            [vec![0x64, 2, 102, 0, 1]]
        );
        assert_eq!(status(), 1);
        lobby
            .db
            .lock()
            .unwrap()
            .execute(
                "UPDATE character_inventory SET count=2 WHERE account='test'",
                [],
            )
            .unwrap();
        handle(&mut socket, peer, &lobby, "test", &mut s, &request(4, 900)).await;
        assert_eq!(status(), 3);
        assert!(s.npc_dialog.is_none());
        s.position = home;
        handle(&mut socket, peer, &lobby, "test", &mut s, &request(7, 900)).await;
        assert_eq!(s.npc_dialog.as_ref().unwrap().choices[0], 2147483002);
        handle(&mut socket, peer, &lobby, "test", &mut s, &[0x55, 0, 0]).await;
        let packets = receive_until(&mut client, 0x4a).await;
        let wallet = packets.last().unwrap();
        assert_eq!(&wallet[..2], &[0x4a, 1]);
        assert_eq!(u32::from_le_bytes(wallet[2..6].try_into().unwrap()), 50000);
        assert_eq!(
            u32::from_le_bytes(wallet[6..10].try_into().unwrap()),
            1050000
        );
        let xp = packets.iter().find(|p| p[0] == 0x1a).unwrap();
        assert_eq!(u32::from_le_bytes(xp[2..6].try_into().unwrap()), 500);
        assert_eq!((status(), s.coins, s.rewards.experience), (2, 1050000, 500));
        assert_eq!(
            crate::quest_rewards::load(&lobby.db.lock().unwrap(), "test", 0)
                .unwrap()
                .experience,
            500
        );
        assert!(
            starter::inventory(&lobby.db.lock().unwrap(), "test", 0)
                .unwrap()
                .iter()
                .all(|i| i.id == 0)
        );
        handle(&mut socket, peer, &lobby, "test", &mut s, &[0x55, 0, 0]).await;
        assert_eq!(
            starter::coins(&lobby.db.lock().unwrap(), "test", 0).unwrap(),
            1050000
        );
    }
    #[tokio::test]
    async fn kaishan_first_job_change_is_paid_once_persisted_and_opens_class_branches() {
        let Some((mut lobby, mut s)) = setup() else {
            return;
        };
        {
            let db = lobby.db.lock().unwrap();
            db.execute("UPDATE world_spawns SET prototype=18004,name='Kaishan'", [])
                .unwrap();
            db.execute("DELETE FROM world_reference_rules", []).unwrap();
            for (id, flag, status, event, exchange) in [
                (24, 0, 0, 100, 0),
                (4061, 406, 0, 403, 460),
                (4063, 406, 2, 110, 460),
                (4065, 406, 4, 403, 460),
            ] {
                let row = serde_json::json!({"sNpcId":"18004","sEventDataIndex":flag,"bEventStatus":status,"bLevel":if flag==0 {1}else{10},"bClass":5,"bNation":3,"bQuestType":1,"nEventTriggerIndex":event,"nExchangeIndex":exchange,"nEventTalkIndex":116,"strLuaFilename":"18004_Kaisan.lua"});
                db.execute(
                    "INSERT INTO world_reference_rules VALUES('QUEST_HELPER',?1,?2)",
                    params![id, row.to_string()],
                )
                .unwrap();
            }
            db.execute("INSERT INTO world_reference_rules VALUES('ITEM_EXCHANGE',460,?1)",[r#"{"nOriginItemNum1":"900000000","nOriginItemCount1":"3000","nExchangeItemNum1":"900006000","nExchangeItemCount1":"1"}"#]).unwrap();
        }
        lobby.merchants =
            crate::shops::load_population(&lobby.db.lock().unwrap(), &reko_common::game_dir())
                .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (mut socket, peer) = listener.accept().await.unwrap();
        for nation in 1..=2u16 {
            for base in 1..=4u16 {
                s.character.class = nation * 100 + base;
                s.character.race = if nation == 1 { base as u8 } else { 12 };
                s.progress.level = 10;
                s.progress.points = [0; 9];
                s.progress.points[0] = 2;
                let bag = s.inventory;
                {
                    let db = lobby.db.lock().unwrap();
                    db.execute(
                        "UPDATE lobby_characters SET class=?1 WHERE account='test'",
                        [s.character.class],
                    )
                    .unwrap();
                    db.execute("DELETE FROM character_quests WHERE account='test'", [])
                        .unwrap();
                    db.execute(
                        "UPDATE character_wallets SET coins=1000000 WHERE account='test'",
                        [],
                    )
                    .unwrap();
                    crate::progress::save(&db, "test", 0, &s.progress).unwrap();
                }
                s.coins = 1000000;
                handle(
                    &mut socket,
                    peer,
                    &lobby,
                    "test",
                    &mut s,
                    &[0x20, 0xe0, 0x2e],
                )
                .await;
                assert_eq!(s.npc_dialog.as_ref().unwrap().choices[0], 2147483001);
                let all = helpers(&lobby.db.lock().unwrap()).unwrap();
                let context = s.npc_dialog.clone().unwrap();
                let script = database_quest_script(&all, &context).unwrap();
                let prompt = execute(
                    &lobby,
                    "test",
                    &s,
                    &lobby.merchants[0],
                    context,
                    (100, -1),
                    &script,
                )
                .unwrap();
                let packet = &prompt.packets[0];
                let talk_id = u32::from_le_bytes(packet[8..12].try_into().unwrap());
                let menu_id = u32::from_le_bytes(packet[12..16].try_into().unwrap());
                let filename_offset = 12 + 12 * 4;
                let filename_length = usize::from(packet[filename_offset]);
                let filename = &packet[filename_offset + 1..];
                assert_eq!(filename_length, filename.len());
                assert_eq!(filename, b"18004_Kaisan.lua");
                let mut confirmation = vec![0x55, 0, filename_length as u8];
                confirmation.extend(filename);
                let game = reko_common::game_dir();
                let talks = reko_assets::Table::from_encrypted_bytes(
                    &std::fs::read(game.join("Data/Quest_Talk_us.tbl")).unwrap(),
                )
                .unwrap();
                let menus = reko_assets::Table::from_encrypted_bytes(
                    &std::fs::read(game.join("Data/Quest_Menu_us.tbl")).unwrap(),
                )
                .unwrap();
                let row = talks
                    .rows
                    .iter()
                    .find(|row| combat::integer(row, 0) == talk_id as i32)
                    .unwrap();
                assert!(
                    matches!(&row[1],reko_assets::CellValue::String(text) if text.contains("job change") && !text.contains("silk"))
                );
                let row = menus
                    .rows
                    .iter()
                    .find(|row| combat::integer(row, 0) == menu_id as i32)
                    .unwrap();
                assert!(
                    matches!(&row[1],reko_assets::CellValue::String(text) if text.to_ascii_lowercase().contains("job change"))
                );
                handle(&mut socket, peer, &lobby, "test", &mut s, &confirmation).await;
                assert_eq!(s.character.class, nation * 100 + base * 2 + 3);
                assert_eq!(s.coins, 997000);
                let packets = receive_until(&mut client, 0x4a).await;
                let packet = packets.last().unwrap();
                assert_eq!(&packet[..2], &[0x4a, 2]);
                assert_eq!(u32::from_le_bytes(packet[2..6].try_into().unwrap()), 3000);
                assert_eq!(
                    u32::from_le_bytes(packet[6..10].try_into().unwrap()),
                    997000
                );
                assert_eq!(s.inventory, bag);
                assert_eq!(s.progress.points[0], 2);
                {
                    let db = lobby.db.lock().unwrap();
                    assert!(crate::quests::completed(&db, "test", 0, 406).unwrap());
                    assert_eq!(
                        db.query_row(
                            "SELECT class FROM lobby_characters WHERE account='test'",
                            [],
                            |r| r.get::<_, u16>(0)
                        )
                        .unwrap(),
                        s.character.class
                    );
                    assert!(
                        crate::progress::allocate(&db, "test", 0, s.character.class, 5)
                            .unwrap()
                            .1
                    );
                }
                handle(&mut socket, peer, &lobby, "test", &mut s, &[0x55, 0, 0]).await;
                assert_eq!(
                    starter::coins(&lobby.db.lock().unwrap(), "test", 0).unwrap(),
                    997000
                );
            }
        }
        s.character.class = 103;
        s.character.race = 3;
        s.progress.level = 10;
        {
            let db = lobby.db.lock().unwrap();
            db.execute("DELETE FROM character_quests WHERE account='test'", [])
                .unwrap();
            db.execute(
                "UPDATE lobby_characters SET class=103 WHERE account='test'",
                [],
            )
            .unwrap();
            db.execute(
                "UPDATE character_wallets SET coins=2999 WHERE account='test'",
                [],
            )
            .unwrap();
        }
        s.coins = 2999;
        handle(
            &mut socket,
            peer,
            &lobby,
            "test",
            &mut s,
            &[0x20, 0xe0, 0x2e],
        )
        .await;
        handle(&mut socket, peer, &lobby, "test", &mut s, &[0x55, 0, 0]).await;
        assert_eq!(s.character.class, 103);
        assert_eq!(
            starter::coins(&lobby.db.lock().unwrap(), "test", 0).unwrap(),
            2999
        );
        assert!(!crate::quests::completed(&lobby.db.lock().unwrap(), "test", 0, 406).unwrap());
        s.progress.level = 9;
        handle(
            &mut socket,
            peer,
            &lobby,
            "test",
            &mut s,
            &[0x20, 0xe0, 0x2e],
        )
        .await;
        assert!(!s.npc_dialog.as_ref().unwrap().choices.contains(&2147483001));
    }
    #[test]
    fn supplied_database_quest_acceptance_audit() {
        let path = std::path::Path::new("../../reko.db");
        if !path.exists() {
            return;
        }
        let Some((mut lobby, mut s)) = crate::world_service_tests::setup() else {
            return;
        };
        let source =
            Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        {
            let db = lobby.db.lock().unwrap();
            let mut q=source.prepare("SELECT table_name,rule_id,data FROM world_reference_rules WHERE table_name IN ('QUEST_HELPER','QUEST_MONSTER','ITEM_EXCHANGE')").unwrap();
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
        {
            let db = lobby.db.lock().unwrap();
            let mut towns = source.prepare("SELECT * FROM world_town_starts").unwrap();
            for row in towns
                .query_map([], |r| {
                    (0..8)
                        .map(|i| r.get::<_, rusqlite::types::Value>(i))
                        .collect::<rusqlite::Result<Vec<_>>>()
                })
                .unwrap()
            {
                db.execute(
                    "INSERT OR REPLACE INTO world_town_starts VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                    rusqlite::params_from_iter(row.unwrap()),
                )
                .unwrap();
            }
        }
        let all = helpers(&lobby.db.lock().unwrap()).unwrap();
        let mut tested = std::collections::HashSet::new();
        let mut report = Vec::new();
        for h in all.iter().filter(|h| h.status == 4 && h.flag != 406) {
            let Some(merchant) = lobby.merchants.iter().find(|m| m.prototype == h.npc) else {
                continue;
            };
            let context = Context {
                npc: merchant.id,
                helper: h.id,
                file: h.file.clone(),
                choices: [-1; 10],
            };
            let Some(script) = database_quest_script(&all, &context) else {
                continue;
            };
            for nation in 1..=2 {
                for (job, class) in [(1, 6), (2, 8), (3, 10), (4, 12)] {
                    s.character.race = if nation == 1 { 2 } else { 12 };
                    s.character.class = nation * 100 + class;
                    if !eligible(h, &s) || !tested.insert((h.flag, nation, job)) {
                        continue;
                    }
                    s.position = merchant.position;
                    let offer = execute(
                        &lobby,
                        "test",
                        &s,
                        merchant,
                        context.clone(),
                        (h.event, -1),
                        &script,
                    )
                    .unwrap();
                    assert_eq!(offer.context.choices[0], 2147483001, "helper{}", h.id);
                    let accepted = execute(
                        &lobby,
                        "test",
                        &s,
                        merchant,
                        offer.context,
                        (2147483001, -1),
                        &script,
                    )
                    .unwrap();
                    assert_eq!(accepted.quests.get(&h.flag), Some(&1), "helper{}", h.id);
                    let db = lobby.db.lock().unwrap();
                    db.execute(
                        "DELETE FROM character_quests WHERE account='test' AND slot=0",
                        [],
                    )
                    .unwrap();
                    report.push(serde_json::json!({"helper":h.id,"flag":h.flag,"nation":nation,"job":job,"npc":h.npc,"result":"accepted"}));
                }
            }
        }
        assert!(report.len() > 100, "expected broad reference-data coverage");
        eprintln!("Reference quest acceptance cases: {}", report.len());
        std::fs::write(
            "../../logs/quest-acceptance-audit.json",
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
    }
    #[tokio::test]
    async fn native_available_state_starts_counts_and_completes_database_quest_once() {
        let Some((mut lobby, mut s)) = setup() else {
            return;
        };
        {
            let db = lobby.db.lock().unwrap();
            db.execute("UPDATE world_spawns SET prototype=13013", [])
                .unwrap();
            for (id, status, event) in [
                (48, 0, 175),
                (49, 1, 185),
                (50, 2, 165),
                (51, 3, 185),
                (52, 4, 175),
            ] {
                let row = serde_json::json!({"sNpcId":"13013","sEventDataIndex":"101","bEventStatus":status.to_string(),"bLevel":"2","bClass":"5","bNation":"3","bQuestType":"1","nExchangeIndex":"5","nEventTalkIndex":"176","nEventTriggerIndex":event.to_string(),"strLuaFilename":"13013_Patric.lua"});
                db.execute(
                    "INSERT INTO world_reference_rules VALUES('QUEST_HELPER',?1,?2)",
                    params![id, row.to_string()],
                )
                .unwrap();
            }
            db.execute(
                "INSERT INTO world_reference_rules VALUES('QUEST_MONSTER',101,?1)",
                [r#"{"sNum1a":"700","sNum1b":"750","sNum1c":"751","sNum1d":"752","sCount1":"5"}"#],
            )
            .unwrap();
            db.execute("INSERT INTO world_reference_rules VALUES('ITEM_EXCHANGE',5,?1)",[r#"{"bRandomFlag":"10","nOriginItemNum1":"900005000","nOriginItemCount1":"101","nExchangeItemNum1":"900001000","nExchangeItemCount1":"50"}"#]).unwrap();
        }
        lobby.merchants =
            crate::shops::load_population(&lobby.db.lock().unwrap(), &reko_common::game_dir())
                .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let _client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (mut socket, peer) = listener.accept().await.unwrap();
        handle(
            &mut socket,
            peer,
            &lobby,
            "test",
            &mut s,
            &[0x64, 7, 52, 0, 0, 0],
        )
        .await;
        assert_eq!(s.npc_dialog.as_ref().unwrap().choices[0], 2147483001);
        handle(&mut socket, peer, &lobby, "test", &mut s, &[0x55, 0, 0]).await;
        let state = || {
            lobby.db.lock().unwrap().query_row("SELECT state FROM character_quests WHERE account='test' AND slot=0 AND quest=101",[],|r|r.get::<_,u8>(0)).unwrap()
        };
        assert_eq!(state(), 1);
        {
            let db = lobby.db.lock().unwrap();
            assert!(crate::quests::kill(&db, "test", 0, 999).unwrap().is_empty());
            for count in 1..=5 {
                assert_eq!(
                    crate::quests::kill(&db, "test", 0, 700).unwrap(),
                    [
                        vec![0x64, 9, 1, 101, 0, count - 1, 0, 0, 0],
                        vec![0x64, 9, 2, 1, count]
                    ]
                );
            }
            assert!(crate::quests::kill(&db, "test", 0, 700).unwrap().is_empty());
            assert_eq!(
                crate::quests::monster_snapshot(&db, "test", 0).unwrap(),
                [0x64, 9, 1, 101, 0, 5, 0, 0, 0]
            );
        }
        let position = (s.position.x, s.position.z);
        s.position.x = 1000;
        s.position.z = 1000;
        s.npc_dialog = None;
        let ready = check_progress(&lobby, "test", &s, &[0x64, 4, 49, 0, 0, 0]).unwrap();
        assert_eq!(ready, [vec![0x64, 2, 101, 0, 3]]);
        assert_eq!(state(), 3);
        assert_eq!(s.rewards.experience, 0);
        assert_eq!(
            crate::quests::monster_snapshot(&lobby.db.lock().unwrap(), "test", 0).unwrap(),
            [0x64, 9, 1, 101, 0, 5, 0, 0, 0]
        );
        assert!(
            check_progress(&lobby, "test", &s, &[0x64, 4, 49, 0, 0, 0])
                .unwrap()
                .is_empty()
        );
        (s.position.x, s.position.z) = position;
        handle(
            &mut socket,
            peer,
            &lobby,
            "test",
            &mut s,
            &[0x64, 7, 49, 0, 0, 0],
        )
        .await;
        assert_eq!(s.npc_dialog.as_ref().unwrap().choices[0], 2147483002);
        handle(&mut socket, peer, &lobby, "test", &mut s, &[0x55, 0, 0]).await;
        assert_eq!(state(), 2);
        assert_eq!(s.coins, 1000000);
        assert_eq!(s.rewards.experience, 50);
        assert_eq!(
            crate::quest_rewards::load(&lobby.db.lock().unwrap(), "test", 0)
                .unwrap()
                .experience,
            50
        );
        assert!(s.inventory.iter().all(|item| item.id == 0));
        handle(&mut socket, peer, &lobby, "test", &mut s, &[0x55, 0]).await;
        assert_eq!(state(), 2);
        assert_eq!(s.coins, 1000000);
        assert_eq!(s.rewards.experience, 50);
        assert!(open_state(4, 0) && open_state(4, 4) && !open_state(4, 2));
        {
            let db = lobby.db.lock().unwrap();
            db.execute(
                "UPDATE character_quests SET state=1 WHERE account='test' AND slot=0 AND quest=101",
                [],
            )
            .unwrap();
            db.execute("UPDATE world_reference_rules SET data=?1 WHERE table_name='ITEM_EXCHANGE' AND rule_id=5",[r#"{"bRandomFlag":"12","nExchangeItemNum1":"110110001","nExchangeItemCount1":"1","nExchangeItemNum2":"160100001","nExchangeItemCount2":"1"}"#]).unwrap();
        }
        handle(
            &mut socket,
            peer,
            &lobby,
            "test",
            &mut s,
            &[0x64, 7, 49, 0, 0, 0],
        )
        .await;
        handle(&mut socket, peer, &lobby, "test", &mut s, &[0x55, 50, 0]).await;
        assert_eq!(state(), 1);
        assert!(s.inventory.iter().all(|item| item.id == 0));
        handle(
            &mut socket,
            peer,
            &lobby,
            "test",
            &mut s,
            &[0x55, 0, 3, b'b', b'a', b'd'],
        )
        .await;
        assert_eq!(state(), 1);
        handle(&mut socket, peer, &lobby, "test", &mut s, &[0x55, 10, 0]).await;
        assert_eq!(state(), 2);
        assert_eq!(
            s.inventory
                .iter()
                .filter(|item| item.id == 160100001)
                .count(),
            1
        );
        assert!(!s.inventory.iter().any(|item| item.id == 110110001));
    }
    fn setup() -> Option<(Lobby, Session)> {
        let (mut l, s) = crate::world_service_tests::setup()?;
        {
            let db = l.db.lock().unwrap();
            crate::population::tests::seed(&db);
            db.execute("UPDATE world_spawns SET monster=0,prototype=16079,name='Menisia',x0=3200,x1=3200,z0=4000,z1=4000",[]).unwrap();
            db.execute("INSERT INTO world_reference_rules VALUES ('QUEST_HELPER',53,?1)",[r#"{"sNpcId":"16079","sEventDataIndex":"0","bEventStatus":"0","bLevel":"1","bClass":"5","bNation":"3","nEventTriggerIndex":"190","strLuaFilename":"16079_Menisia.lua   "}"#]).unwrap();
        }
        l.merchants =
            crate::shops::load_population(&l.db.lock().unwrap(), &reko_common::game_dir()).unwrap();
        Some((l, s))
    }
    #[test]
    fn reference_menu_and_atomic_rollback_preserve_inventory_and_coins() {
        let Some((l, s)) = setup() else {
            return;
        };
        let h = helpers(&l.db.lock().unwrap()).unwrap().remove(0);
        assert_eq!(h.file, "16079_Menisia.lua");
        let context = Context {
            npc: 12000,
            helper: 53,
            file: h.file,
            choices: [-1; 10],
        };
        let path = reko_common::game_dir()
            .parent()
            .unwrap()
            .join("kn_2086_patch_private_server/lCAFILES/Quests/16079_Menisia.lua");
        if path.exists() {
            let result = execute(
                &l,
                "test",
                &s,
                &l.merchants[0],
                context.clone(),
                (190, -1),
                &std::fs::read(path).unwrap(),
            )
            .unwrap();
            assert_eq!(result.packets[0][0], 0x55);
            assert_eq!(result.context.choices[0], 193);
            assert_eq!(result.coins, 1000000);
        }
        for script in [
            &b"GoldLose(UID, 100); GiveItem(UID, 110110001, 1); UnsupportedOperation(UID)"[..],
            &b"GoldLose(UID, 100); while true do end"[..],
            &b"GoldLose(UID, 100); GiveItem(UID, 999999999, 1)"[..],
        ] {
            assert!(
                execute(
                    &l,
                    "test",
                    &s,
                    &l.merchants[0],
                    context.clone(),
                    (190, -1),
                    script
                )
                .is_err()
            );
            let db = l.db.lock().unwrap();
            assert_eq!(starter::coins(&db, "test", 0).unwrap(), 1000000);
            assert!(
                starter::inventory(&db, "test", 0)
                    .unwrap()
                    .iter()
                    .all(|i| i.id == 0)
            );
        }
        let result = execute(
            &l,
            "test",
            &s,
            &l.merchants[0],
            context,
            (190, -1),
            b"GoldLose(UID,100); GiveItem(UID,110110001,1)",
        )
        .unwrap();
        assert_eq!(result.coins, 999900);
        assert_eq!(result.bag[0].id, 110110001);
        assert_eq!(result.context.choices, [-1; 10]);
        let db = l.db.lock().unwrap();
        assert_eq!(starter::coins(&db, "test", 0).unwrap(), 999900);
        assert_eq!(starter::inventory(&db, "test", 0).unwrap()[0].id, 110110001);
    }
    #[test]
    fn supplied_default_npc_script_coverage_audit() {
        let source = std::path::Path::new("../../logs/allzones-import-test.db");
        if !source.exists() {
            return;
        }
        let Some((mut lobby, s)) = crate::world_service_tests::setup() else {
            return;
        };
        let original =
            Connection::open_with_flags(source, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .unwrap();
        {
            let db = lobby.db.lock().unwrap();
            let mut q=original.prepare("SELECT table_name,rule_id,data FROM world_reference_rules WHERE table_name IN ('QUEST_HELPER','ITEM_EXCHANGE')").unwrap();
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
                    "INSERT OR REPLACE INTO world_reference_rules VALUES(?1,?2,?3)",
                    params![t, id, data],
                )
                .unwrap();
            }
        }
        lobby.merchants =
            crate::shops::load_population(&original, &reko_common::game_dir()).unwrap();
        let all = helpers(&lobby.db.lock().unwrap()).unwrap();
        let mut report = Vec::new();
        for h in all
            .iter()
            .filter(|h| h.flag == 0 && h.status == 0 && eligible(h, &s))
        {
            let Some(m) = lobby.merchants.iter().find(|m| m.prototype == h.npc) else {
                continue;
            };
            let path = reko_common::game_dir()
                .parent()
                .unwrap()
                .join("kn_2086_patch_private_server/lCAFILES/Quests")
                .join(&h.file);
            let context = Context {
                npc: m.id,
                helper: h.id,
                file: h.file.clone(),
                choices: [-1; 10],
            };
            let result = std::fs::read(path)
                .map_err(anyhow::Error::from)
                .and_then(|script| execute(&lobby, "test", &s, m, context, (h.event, -1), &script));
            let status = match result {
                Ok(r) if !r.packets.is_empty() => "menu".to_string(),
                Ok(_) => "no matching event in supplied script".to_string(),
                Err(e) => e.to_string(),
            };
            report.push(serde_json::json!({"helper":h.id,"npc":h.npc,"file":h.file,"event":h.event,"status":status}));
        }
        let covered = report.iter().filter(|r| r["status"] == "menu").count();
        eprintln!(
            "Default NPC coverage: {covered}/{} supplied script events",
            report.len()
        );
        std::fs::write(
            "../../logs/npc-script-coverage.json",
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
        assert!(
            covered > 10,
            "reference menus must execute across multiple NPCs"
        );
    }

    #[tokio::test]
    async fn menu_choices_and_interaction_range_are_server_authoritative() {
        let Some((l, mut s)) = setup() else {
            return;
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let _client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (mut socket, peer) = listener.accept().await.unwrap();
        handle(&mut socket, peer, &l, "test", &mut s, &[0x55, 0]).await;
        assert!(s.npc_dialog.is_none());
        handle(&mut socket, peer, &l, "test", &mut s, &[0x20, 0xe0, 0x2e]).await;
        assert!(s.npc_dialog.is_some());
        s.position.x = 500;
        handle(&mut socket, peer, &l, "test", &mut s, &[0x55, 0]).await;
        assert_eq!(
            starter::coins(&l.db.lock().unwrap(), "test", 0).unwrap(),
            1000000
        );
    }
}
