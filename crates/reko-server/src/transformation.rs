//! Original Transformation Scroll list, paid selection and persisted disguise.
use crate::{combat, lobby::Lobby, magic, resources, skills, world_entry::Session};
use std::net::SocketAddr;
use tokio::net::TcpStream;

pub(crate) const CATEGORY: u8 = 96;
const LIST: u32 = 472001;
const SCROLL: u32 = 381001000;

#[cfg(test)]
#[path = "transformation_tests.rs"]
mod tests;

fn permitted_zone(zone: u8) -> bool {
    !matches!(zone, 31 | 48 | 51..=69 | 71..=75)
}

pub(crate) fn npc_form(id: u32) -> bool {
    (500505..=500512).contains(&id)
}

fn permitted_form_zone(id: u32, zone: u8) -> bool {
    npc_form(id) || permitted_zone(zone)
}

pub(crate) fn appearance(s: &Session) -> u32 {
    s.skills
        .buffs
        .get(&CATEGORY)
        .filter(|b| {
            b.until > skills::now_ms()
                && s.skills.hp > 0
                && permitted_form_zone(b.skill, s.position.zone)
        })
        .map_or(1, |b| b.skill)
}

fn usable(
    s: &Session,
    catalog: &skills::Catalog,
    state: &skills::State,
    npc: bool,
) -> Result<(), &'static str> {
    if s.zone_loading || state.hp == 0 || !npc && !permitted_zone(s.position.zone) {
        return Err("You cannot transform in this zone or while unavailable.");
    }
    if state.buffs.iter().any(|(kind, b)| {
        b.until > skills::now_ms()
            && (*kind == CATEGORY
                || !npc
                    && (*kind == 91
                        || catalog.spells.get(&b.skill).is_some_and(|spell| {
                            spell.types[0] == 4 && !matches!(spell.moral, 7 | 10 | 12)
                        })))
    }) {
        return Err("Remove active buffs before using a Transformation Scroll.");
    }
    Ok(())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Rule {
    #[serde(rename = "TransformID")]
    transform_id: u16,
    duration: u16,
    class: u16,
    user_skill_use: u8,
    nation: u8,
}

pub(crate) async fn refresh(
    sock: &mut TcpStream,
    peer: SocketAddr,
    lobby: &Lobby,
    account: &str,
    s: &mut Session,
) {
    if s.skills.buffs.contains_key(&CATEGORY)
        && (s.skills.hp == 0
            || s.skills
                .buffs
                .get(&CATEGORY)
                .is_some_and(|b| !permitted_form_zone(b.skill, s.position.zone)))
    {
        let result = {
            let db = lobby.db.lock().unwrap();
            skills::load(&db, account, s.character.slot).and_then(|mut next| {
                next.buffs.remove(&CATEGORY);
                skills::save(&db, account, s.character.slot, &next)?;
                Ok(next)
            })
        };
        let Ok(next) = result else { return };
        s.skills = next;
    }
    let form = appearance(s);
    if s.transformation_visual == form {
        return;
    }
    if form == 1 {
        crate::send_response(sock, peer, &None, &[0x31, 7], "TRANSFORMATION_ENDED").await;
    }
    if form != 1 && s.is_admin {
        let mut visible = vec![0x29];
        visible.extend(s.player_id.to_le_bytes());
        visible.push(5);
        visible.extend(1u32.to_le_bytes());
        crate::send_response(sock, peer, &None, &visible, "TRANSFORMATION_GM_VISIBLE").await;
    }
    let mut packet = vec![0x29];
    packet.extend(s.player_id.to_le_bytes());
    packet.push(3);
    packet.extend(form.to_le_bytes());
    crate::send_response(sock, peer, &None, &packet, "TRANSFORMATION_APPEARANCE").await;
    if form != 1 {
        let mut effect = skills::effect(form, s.player_id);
        effect[16..18].copy_from_slice(
            &(s.skills.buffs[&CATEGORY]
                .until
                .saturating_sub(skills::now_ms())
                .div_ceil(1000)
                .min(32767) as i16)
                .to_le_bytes(),
        );
        crate::send_response(sock, peer, &None, &effect, "TRANSFORMATION_EFFECT").await;
    }
    let mut hub = lobby.social.lock().unwrap();
    let peers = hub
        .players
        .iter()
        .filter(|(id, p)| {
            **id != s.player_id
                && p.s.entered
                && crate::population::same_region_view(s.position, p.s.position)
        })
        .map(|(id, _)| *id)
        .collect::<Vec<_>>();
    hub.send_many(&peers, &packet);
    s.transformation_visual = form;
}

pub(crate) async fn handle(
    sock: &mut TcpStream,
    peer: SocketAddr,
    lobby: &Lobby,
    account: &str,
    s: &mut Session,
    p: &[u8],
) -> bool {
    if p.len() != 22 {
        return false;
    }
    let id = u32::from_le_bytes(p[2..6].try_into().unwrap());
    if id != LIST && !(472010..=472999).contains(&id) && !npc_form(id) {
        return false;
    }
    let Some(catalog) = lobby.skills.as_ref() else {
        return false;
    };
    let Some(spell) = catalog.spells.get(&id) else {
        return false;
    };
    let caster = u16::from_le_bytes([p[6], p[7]]);
    let target = u16::from_le_bytes([p[8], p[9]]);
    let result = (|| -> Result<Option<resources::Paid>, &'static str> {
        if caster != s.player_id || ![s.player_id, u16::MAX].contains(&target) {
            return Err("Transformation requires your own character.");
        }
        if p[1] == 6 && id != LIST {
            if s.skills.buffs.get(&CATEGORY).is_some_and(|b| b.skill == id) {
                let db = lobby.db.lock().unwrap();
                let mut next = skills::load(&db, account, s.character.slot)
                    .map_err(|_| "Unable to load transformation.")?;
                next.buffs.remove(&CATEGORY);
                skills::save(&db, account, s.character.slot, &next)
                    .map_err(|_| "Unable to cancel transformation.")?;
                s.skills = next;
            }
            return Ok(None);
        }
        if p[1] != 3 {
            return Err("Unsupported transformation phase.");
        }
        let npc = npc_form(id);
        usable(s, catalog, &s.skills, npc)?;
        if npc
            && (spell.types != [6, 4]
                || spell.skill_group != 0
                || spell.moral != 1
                || !lobby
                    .combat
                    .as_ref()
                    .is_some_and(|items| items.usable_item_skill(spell, s.progress.level)))
        {
            return Err("Invalid NPC transformation item definition.");
        }
        if id == LIST {
            if spell.use_item != SCROLL || spell.types != [0, 1] {
                return Err("Invalid Transformation Scroll definition.");
            }
            let bag =
                crate::starter::inventory(&lobby.db.lock().unwrap(), account, s.character.slot)
                    .map_err(|_| "Unable to check Transformation Scroll.")?;
            if !bag.iter().any(|i| i.id == SCROLL && i.count > 0) {
                return Err("You need a Transformation Scroll.");
            }
            s.pending_cast = Some(magic::Cast {
                skill: LIST,
                target: s.player_id,
                expires_at: skills::now_ms() + 60000,
            });
            return Ok(None);
        }
        if spell.types[0] != 6
            || !npc
                && !s
                    .pending_cast
                    .as_ref()
                    .is_some_and(|c| c.matches(LIST, s.player_id, skills::now_ms()))
        {
            return Err("Open the Transformation Scroll list first.");
        }
        if u16::from(s.progress.level) < spell.required_points {
            return Err("Your level is too low for this transformation.");
        }
        let row = catalog
            .effects
            .get(&(6, id))
            .ok_or("Transformation is missing from the original1453 table.")?;
        let rule: Rule = if npc {
            if (6..20).any(|col| combat::integer(row, col) != 0) {
                return Err("Unsupported NPC transformation combat modifiers.");
            }
            Rule {
                transform_id: combat::integer(row, 4) as u16,
                duration: combat::integer(row, 5) as u16,
                class: combat::integer(row, 21) as u16,
                user_skill_use: combat::integer(row, 22) as u8,
                nation: combat::integer(row, 28) as u8,
            }
        } else {
            let text:String=lobby.db.lock().unwrap().query_row("SELECT data FROM world_reference_rules WHERE table_name='MAGIC_TYPE6' AND rule_id=?1",[id],|r|r.get(0)).map_err(|_|"This transformation has no supplied1453 rule.")?;
            serde_json::from_str(&text).map_err(|_| "Invalid transformation rule.")?
        };
        let job = match s.character.class % 100 {
            1 | 5 | 6 => 0,
            2 | 7 | 8 => 1,
            3 | 9 | 10 => 2,
            4 | 11 | 12 => 3,
            _ => return Err("Unsupported transformation class."),
        };
        if rule.user_skill_use != if npc { 3 } else { 1 }
            || rule.duration == 0
            || rule.nation != 0 && rule.nation != s.character.nation()
            || rule.class != 0 && rule.class / 10u16.pow(3 - job) % 10 != 1
            || combat::integer(row, 4) != i32::from(rule.transform_id)
            || combat::integer(row, 5) != i32::from(rule.duration)
        {
            return Err("This transformation is unavailable for your character.");
        }
        let mut paid_spell = spell.clone();
        if !npc {
            paid_spell.use_item = SCROLL;
        }
        let paid = resources::commit(lobby, account, s.character.slot, &paid_spell, 1, |state| {
            usable(s, catalog, state, npc)?;
            state.buffs.insert(
                CATEGORY,
                skills::Buff {
                    skill: id,
                    until: skills::now_ms() + u64::from(rule.duration) * 1000,
                },
            );
            Ok(())
        })?;
        s.pending_cast = None;
        Ok(Some(paid))
    })();
    match result {
        Ok(Some(paid)) => {
            resources::sync(sock, peer, lobby, account, s, paid).await;
            refresh(sock, peer, lobby, account, s).await;
        }
        Ok(None) if id == LIST && p[1] == 3 => {
            let mut packet = vec![0x31, 9];
            packet.extend(LIST.to_le_bytes());
            crate::send_response(sock, peer, &None, &packet, "TRANSFORMATION_LIST").await;
        }
        Ok(None) => refresh(sock, peer, lobby, account, s).await,
        Err(reason) => magic::refused(sock, peer, s, id, reason).await,
    }
    true
}
