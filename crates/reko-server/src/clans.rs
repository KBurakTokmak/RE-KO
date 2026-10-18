//! Nation-specific persistent clans and original membership packets.
use crate::{
    lobby::Lobby,
    social::{Hub, name, near, string},
    world_entry::Session,
};
use rusqlite::{Connection, OptionalExtension, params};
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Info {
    pub id: u16,
    pub name: String,
    pub fame: u8,
    pub grade: u8,
}
pub(crate) fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(include_str!("clans.sql"))
}
pub(crate) fn load(db: &Connection, account: &str, slot: u8) -> rusqlite::Result<Info> {
    Ok(db.query_row("SELECT c.id,c.name,m.fame FROM clan_members m JOIN clans c ON c.id=m.clan_id WHERE m.account=?1 AND m.slot=?2",params![account,slot],|r|Ok(Info{id:r.get(0)?,name:r.get(1)?,fame:r.get(2)?,grade:5})).optional()?.unwrap_or_default())
}
fn member_count(db: &Connection, id: u16) -> Result<u16, u8> {
    db.query_row(
        "SELECT count(*) FROM clan_members WHERE clan_id=?1",
        [id],
        |r| r.get(0),
    )
    .map_err(|_| 0)
}
fn members(h: &Hub, clan: u16) -> Vec<u16> {
    h.players
        .iter()
        .filter(|(_, p)| p.s.clan.id == clan)
        .map(|(&id, _)| id)
        .collect()
}
fn changed(h: &mut Hub, s: &Session, packet: Vec<u8>) {
    let ids = h
        .players
        .iter()
        .filter(|(id, p)| {
            **id == s.player_id || crate::population::same_region_view(p.s.position, s.position)
        })
        .map(|(&id, _)| id)
        .collect::<Vec<_>>();
    h.send_many(&ids, &packet);
}
fn role_packet(sub: u8, id: u16, clan: u16, fame: u8) -> Vec<u8> {
    let mut p = vec![0x3c, sub, 1];
    p.extend(id.to_le_bytes());
    p.extend(clan.to_le_bytes());
    p.push(fame);
    p
}
fn roster(db: &Connection, h: &Hub, s: &Session) -> Result<Vec<u8>, u8> {
    if s.clan.id == 0 {
        return Err(7);
    }
    let mut q=db.prepare("SELECT c.name,m.fame,COALESCE(p.level,1),c.class FROM clan_members m JOIN lobby_characters c USING(account,slot) LEFT JOIN character_progress p USING(account,slot) WHERE m.clan_id=?1 ORDER BY m.fame,c.name").map_err(|_|0)?;
    let rows = q
        .query_map([s.clan.id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, u8>(1)?,
                r.get::<_, u8>(2)?,
                r.get::<_, u16>(3)?,
            ))
        })
        .map_err(|_| 0)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| 0)?;
    let mut p = vec![0x3c, 0x0d, 1];
    let count = rows.len() as u16;
    let online = rows.iter().filter(|r| h.named(&r.0).is_some()).count() as u16;
    for n in [0, online, count, count] {
        p.extend(n.to_le_bytes());
    }
    for (name_, fame, level, class) in rows {
        string(&mut p, &name_);
        p.extend([fame, level]);
        p.extend(class.to_le_bytes());
        p.push(u8::from(h.named(&name_).is_some()));
    }
    let size = (p.len() - 5) as u16;
    p[3..5].copy_from_slice(&size.to_le_bytes());
    Ok(p)
}
fn apply(
    db: &Connection,
    h: &mut Hub,
    account: &str,
    s: &mut Session,
    sub: u8,
    args: &[u8],
) -> Result<(), u8> {
    if s.skills.hp == 0 || s.zone_loading {
        return Err(3);
    }
    s.clan = load(db, account, s.character.slot).map_err(|_| 0)?;
    match sub {
        1 => {
            if s.clan.id != 0 {
                return Err(5);
            }
            if s.position.zone != s.character.nation() {
                return Err(9);
            }
            if s.progress.level < 20 {
                return Err(2);
            }
            let Some(text) = args
                .get(..2)
                .and_then(|v| v.try_into().ok())
                .map(u16::from_le_bytes)
                .filter(|n| usize::from(*n) + 2 == args.len())
                .and_then(|_| std::str::from_utf8(&args[2..]).ok())
            else {
                return Err(3);
            };
            if text.is_empty()
                || text.len() > 20
                || text.trim() != text
                || !text.bytes().all(|b| (0x20..0x7f).contains(&b))
            {
                return Err(3);
            }
            let tx = db.unchecked_transaction().map_err(|_| 1)?;
            let duplicate: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM clans WHERE name=?1)",
                    [text],
                    |r| r.get(0),
                )
                .map_err(|_| 1)?;
            if duplicate {
                return Err(3);
            }
            let nation = s.character.nation();
            let minimum = if nation == 1 { 0 } else { 15000 };
            let maximum = if nation == 1 { 14999 } else { 30000 };
            let id: u16 = tx
                .query_row(
                    "SELECT COALESCE(MAX(id),?2)+1 FROM clans WHERE nation=?1",
                    params![nation, minimum],
                    |r| r.get(0),
                )
                .map_err(|_| 6)?;
            if id > maximum {
                return Err(6);
            }
            let before = crate::starter::coins(&tx, account, s.character.slot).map_err(|_| 1)?;
            if before < 500000 {
                return Err(4);
            }
            tx.execute(
                "INSERT INTO clans VALUES(?1,?2,?3)",
                params![id, text, nation],
            )
            .map_err(|_| 1)?;
            tx.execute(
                "INSERT INTO clan_members VALUES(?1,?2,?3,1)",
                params![account, s.character.slot, id],
            )
            .map_err(|_| 1)?;
            if tx.execute("UPDATE character_wallets SET coins=coins-500000 WHERE account=?1 AND slot=?2 AND coins>=500000",params![account,s.character.slot]).map_err(|_|1)?!=1{return Err(4);}
            tx.commit().map_err(|_| 1)?;
            s.coins = before - 500000;
            s.clan = Info {
                id,
                name: text.into(),
                fame: 1,
                grade: 5,
            };
            let mut p = vec![0x3c, 1, 1];
            p.extend(s.player_id.to_le_bytes());
            p.extend(id.to_le_bytes());
            string(&mut p, text);
            p.extend([5, 0]);
            p.extend(s.coins.to_le_bytes());
            changed(h, s, p);
        }
        2 if args.len() == 2 => {
            if s.clan.id == 0 || !matches!(s.clan.fame, 1 | 2) {
                return Err(6);
            }
            let target = u16::from_le_bytes(args.try_into().unwrap());
            let other = &h.players.get(&target).ok_or(2)?.s;
            if target == s.player_id {
                return Err(9);
            }
            if other.skills.hp == 0 {
                return Err(3);
            }
            if other.character.nation() != s.character.nation() {
                return Err(4);
            }
            if load(db, &h.players[&target].account, other.character.slot)
                .map_err(|_| 0)?
                .id
                != 0
            {
                return Err(5);
            }
            if !near(s.position, other.position, 50) || h.clan_invites.contains_key(&target) {
                return Err(2);
            }
            if member_count(db, s.clan.id)? >= 36 {
                return Err(8);
            }
            let mut p = vec![0x3c, 0x11, 1];
            p.extend(s.player_id.to_le_bytes());
            p.extend(s.clan.id.to_le_bytes());
            string(&mut p, &s.clan.name);
            h.send(target, p);
            h.clan_invites.insert(
                target,
                (s.player_id, s.clan.id, crate::skills::now_ms() + 60000),
            );
        }
        0x11 if args.len() == 5 => {
            let leader = u16::from_le_bytes(args[1..3].try_into().unwrap());
            let clan = u16::from_le_bytes(args[3..5].try_into().unwrap());
            let Some((inviter, invited_clan, until)) = h.clan_invites.get(&s.player_id).copied()
            else {
                return Err(6);
            };
            if inviter != leader || invited_clan != clan || crate::skills::now_ms() >= until {
                return Err(6);
            }
            h.clan_invites.remove(&s.player_id);
            if args[0] != 1 {
                h.send(leader, vec![0x3c, 2, 11]);
                return Ok(());
            }
            if s.clan.id != 0 {
                return Err(5);
            }
            let actor = h.players.get(&leader).ok_or(2)?;
            let info = load(db, &actor.account, actor.s.character.slot).map_err(|_| 0)?;
            if info.id != clan || !matches!(info.fame, 1 | 2) {
                return Err(6);
            }
            if actor.s.character.nation() != s.character.nation() {
                return Err(4);
            }
            if !near(actor.s.position, s.position, 50) || actor.s.skills.hp == 0 {
                return Err(3);
            }
            let tx = db.unchecked_transaction().map_err(|_| 0)?;
            if member_count(&tx, clan)? >= 36 {
                return Err(8);
            }
            tx.execute(
                "INSERT INTO clan_members VALUES(?1,?2,?3,4)",
                params![account, s.character.slot, clan],
            )
            .map_err(|_| 0)?;
            tx.commit().map_err(|_| 0)?;
            s.clan = Info { fame: 4, ..info };
            let mut p = role_packet(2, s.player_id, clan, 4);
            string(&mut p, &s.clan.name);
            p.extend([s.clan.grade, 0]);
            changed(h, s, p);
        }
        3 if args.is_empty() => {
            if s.clan.id == 0 {
                return Err(10);
            }
            if s.clan.fame == 1 {
                return disband(db, h, account, s);
            }
            db.execute(
                "DELETE FROM clan_members WHERE account=?1 AND slot=?2",
                params![account, s.character.slot],
            )
            .map_err(|_| 0)?;
            s.clan = Info::default();
            changed(h, s, role_packet(3, s.player_id, 0, 0));
        }
        5 if args.is_empty() => return disband(db, h, account, s),
        0x0d if args.is_empty() => {
            let p = roster(db, h, s)?;
            h.send(s.player_id, p);
        }
        4 | 6 | 7 | 8 | 9 | 10 | 11 => {
            if s.clan.id == 0 || s.clan.fame != 1 {
                return Err(6);
            }
            let target_name = name(args).ok_or(2)?;
            let row:Option<(String,u8,u8)>=db.query_row("SELECT m.account,m.slot,m.fame FROM clan_members m JOIN lobby_characters c USING(account,slot) WHERE m.clan_id=?1 AND c.name=?2",params![s.clan.id,target_name],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|_|0)?;
            let (target_account, slot, _fame) = row.ok_or(10)?;
            if target_account.eq_ignore_ascii_case(account) && slot == s.character.slot {
                return Err(9);
            }
            let tx = db.unchecked_transaction().map_err(|_| 0)?;
            let fame = match sub {
                4 | 7 => 0,
                6 => 5,
                8 => 3,
                9 => 1,
                10 => 2,
                _ => 6,
            };
            if sub == 10 {
                let count: u8 = tx
                    .query_row(
                        "SELECT count(*) FROM clan_members WHERE clan_id=?1 AND fame=2",
                        [s.clan.id],
                        |r| r.get(0),
                    )
                    .map_err(|_| 0)?;
                if count >= 3 {
                    return Err(8);
                }
            }
            if fame == 0 {
                tx.execute(
                    "DELETE FROM clan_members WHERE account=?1 AND slot=?2",
                    params![target_account, slot],
                )
                .map_err(|_| 0)?;
            } else {
                if sub == 9 {
                    tx.execute(
                        "UPDATE clan_members SET fame=2 WHERE account=?1 AND slot=?2",
                        params![account, s.character.slot],
                    )
                    .map_err(|_| 0)?;
                }
                tx.execute(
                    "UPDATE clan_members SET fame=?3 WHERE account=?1 AND slot=?2",
                    params![target_account, slot, fame],
                )
                .map_err(|_| 0)?;
            }
            tx.commit().map_err(|_| 0)?;
            if sub == 9 {
                s.clan.fame = 2;
                changed(h, s, role_packet(0x10, s.player_id, s.clan.id, 2));
            }
            if let Some(target) = h.named(target_name) {
                let other = h.players[&target].s.clone();
                changed(
                    h,
                    &other,
                    role_packet(
                        if sub == 4 { 4 } else { 0x10 },
                        target,
                        if fame == 0 { 0 } else { s.clan.id },
                        fame,
                    ),
                );
            } else {
                let p = roster(db, h, s)?;
                h.send(s.player_id, p);
            }
        }
        _ => return Err(6),
    }
    Ok(())
}
fn disband(db: &Connection, h: &mut Hub, account: &str, s: &mut Session) -> Result<(), u8> {
    if s.clan.id == 0 || s.clan.fame != 1 {
        return Err(6);
    }
    let tx = db.unchecked_transaction().map_err(|_| 0)?;
    let ids = members(h, s.clan.id);
    tx.execute("DELETE FROM clan_members WHERE clan_id=?1", [s.clan.id])
        .map_err(|_| 0)?;
    tx.execute("DELETE FROM clans WHERE id=?1", [s.clan.id])
        .map_err(|_| 0)?;
    tx.commit().map_err(|_| 0)?;
    h.clan_invites.retain(|_, (_, clan, _)| *clan != s.clan.id);
    for id in ids {
        let other = h.players[&id].s.clone();
        changed(h, &other, role_packet(3, id, 0, 0));
        h.send(id, vec![0x3c, 5, 1]);
    }
    s.clan = load(db, account, s.character.slot).map_err(|_| 0)?;
    Ok(())
}
pub(crate) fn process(lobby: &Lobby, account: &str, s: &mut Session, p: &[u8]) {
    let Some((&sub, args)) = p.get(1..).and_then(|v| v.split_first()) else {
        return;
    };
    let db = lobby.db.lock().unwrap();
    let mut h = lobby.social.lock().unwrap();
    h.clan_invites
        .retain(|_, (_, _, until)| crate::skills::now_ms() < *until);
    if let Err(code) = apply(&db, &mut h, account, s, sub, args) {
        h.send(
            s.player_id,
            vec![0x3c, if sub == 0x11 { 2 } else { sub }, code],
        );
    }
}
