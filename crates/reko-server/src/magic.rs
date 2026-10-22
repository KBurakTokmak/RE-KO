//! Validated skill phases and physical effects against training targets.

use crate::{
    chat, combat, lobby::Lobby, monsters, resources, skills, utility, world_entry::Session,
};
use std::net::SocketAddr;
use tokio::net::TcpStream;

#[derive(Clone)]
pub(crate) struct Cast {
    pub skill: u32,
    pub target: u16,
    pub expires_at: u64,
}

impl Cast {
    pub(crate) fn matches(&self, skill: u32, target: u16, now: u64) -> bool {
        self.skill == skill && self.target == target && now <= self.expires_at
    }
}

pub(crate) fn visibility(lobby: &Lobby, session: &Session, invisible: bool) {
    let mut packet = vec![0x29];
    packet.extend(session.player_id.to_le_bytes());
    packet.push(7);
    packet.extend(u32::from(invisible).to_le_bytes());
    if let Some(world) = lobby.monsters.as_ref() {
        let _ = world.events.send(monsters::Event {
            zone: session.position.zone,
            packet,
        });
    }
}

pub(crate) fn remove_stealth(
    lobby: &Lobby,
    account: &str,
    session: &mut Session,
    movement: bool,
) -> rusqlite::Result<bool> {
    let Some(buff) = session.skills.buffs.get(&91) else {
        return Ok(false);
    };
    if movement
        && !lobby
            .skills
            .as_ref()
            .and_then(|c| c.effects.get(&(9, buff.skill)))
            .is_some_and(|row| combat::integer(row, 7) == 1)
    {
        return Ok(false);
    }
    let db = lobby.db.lock().unwrap();
    let mut next = skills::load(&db, account, session.character.slot)?;
    next.buffs.remove(&91);
    skills::save(&db, account, session.character.slot, &next)?;
    session.skills = next;
    visibility(lobby, session, false);
    Ok(true)
}

pub(crate) async fn refused(
    sock: &mut TcpStream,
    peer: SocketAddr,
    session: &Session,
    id: u32,
    reason: &str,
) {
    let mut failure = skills::effect(id, session.player_id);
    failure[1] = 4;
    failure[10..].fill(0);
    super::send_response(sock, peer, &None, &failure, "SKILL_FAILED").await;
    let notice = chat::packet(
        7,
        session.character.nation(),
        u16::MAX,
        "",
        reason.as_bytes(),
    );
    super::send_response(sock, peer, &None, &notice, "SKILL_NOTICE").await;
    tracing::info!(%peer,id,reason,"skill refused");
}

pub(crate) async fn handle(
    sock: &mut TcpStream,
    peer: SocketAddr,
    payload: &[u8],
    lobby: &Lobby,
    account: Option<&str>,
    session: Option<&mut Session>,
) -> bool {
    let (Some(session), Some(account), Some(catalog)) = (
        session.filter(|s| s.entered),
        account,
        lobby.skills.as_ref(),
    ) else {
        return false;
    };
    if payload.len() != 22 {
        return false;
    }
    let id = u32::from_le_bytes(payload[2..6].try_into().unwrap());
    let Some(spell) = catalog.spells.get(&id) else {
        return false;
    };
    let caster = u16::from_le_bytes([payload[6], payload[7]]);
    let target = u16::from_le_bytes([payload[8], payload[9]]);
    let now = skills::now_ms();
    let failure = if caster != session.player_id {
        Some("Skill caster does not match this session.")
    } else if !spell.learned_by(
        session.character.class,
        session.progress.level,
        &session.progress.points,
    ) {
        Some("This skill's class or learning requirements are not met.")
    } else if session.skills.hp == 0 {
        Some("You cannot use skills while dead.")
    } else if spell.skill_group == 0
        && !lobby.combat.as_ref().is_some_and(|items| {
            items.item(spell.use_item).is_some_and(|(row, extension)| {
                extension.is_some()
                    && combat::integer(row, 10) == 97
                    && combat::integer(row, 24) as u32 == spell.id
            })
        })
    {
        Some("This item does not provide a usable recovery skill.")
    } else if !crate::quests::completed(
        &lobby.db.lock().unwrap(),
        account,
        session.character.slot,
        spell.required_quest,
    )
    .unwrap_or(false)
    {
        Some("Complete this skill's unlock quest before using it.")
    } else if spell.item_group == 1 && session.combat.weapon_kind / 10 != 1 {
        Some("This skill requires an equipped dagger.")
    } else if spell.item_group == 7 && !matches!(session.combat.weapon_kind / 10, 7 | 8 | 10) {
        Some("This skill requires an equipped bow.")
    } else {
        None
    };
    if let Some(reason) = failure {
        refused(sock, peer, session, id, reason).await;
        return true;
    }
    if payload[1] == 6 && target == caster {
        if spell.types[0] == 4 && matches!(spell.moral, 7 | 10 | 12) {
            refused(
                sock,
                peer,
                session,
                id,
                "Hostile effects require a cure or must expire; they cannot be cancelled.",
            )
            .await;
            return true;
        }
        let category = session
            .skills
            .buffs
            .iter()
            .find(|(_, buff)| buff.skill == id)
            .map(|(kind, _)| *kind);
        if let Some(category) = category {
            let result = {
                let db = lobby.db.lock().unwrap();
                skills::load(&db, account, session.character.slot).and_then(|mut next| {
                    next.buffs.remove(&category);
                    if matches!(category, 1 | 7) {
                        let stats = utility::effective(
                            session,
                            &next,
                            Some(catalog),
                            lobby.combat.as_ref(),
                        );
                        next.set_limits(stats.hp, stats.mp);
                    }
                    skills::save(&db, account, session.character.slot, &next)?;
                    Ok(next)
                })
            };
            if let Ok(next) = result {
                session.skills = next;
                utility::refresh(session, Some(catalog), lobby.combat.as_ref());
                if category == 91 {
                    visibility(lobby, session, false);
                }
                super::send_response(sock, peer, &None, &[0x31, 5, category], "BUFF_CANCELLED")
                    .await;
                super::send_response(
                    sock,
                    peer,
                    &None,
                    &crate::inventory::attributes(&session.combat, 2),
                    "BUFF_ATTRIBUTES",
                )
                .await;
            }
        }
        return true;
    }
    session.sitting = false;
    if spell.types[0] == 4
        && catalog
            .effects
            .get(&(4, id))
            .is_some_and(|row| combat::integer(row, 1) == 32)
    {
        refused(sock, peer, session, id, "Armor Cancellation affects enemy players, not monsters. Player combat is not implemented yet.").await;
        return true;
    }
    if spell.types[0] == 2 {
        if let Err(reason) =
            crate::archery::handle(sock, peer, lobby, account, session, spell, payload).await
        {
            refused(sock, peer, session, id, reason).await;
        }
        return true;
    }
    if let Some(result) =
        crate::mage::handle(sock, peer, lobby, account, session, spell, payload).await
    {
        if let Err(reason) = result {
            refused(sock, peer, session, id, reason).await;
        }
        return true;
    }
    if payload[1] == 1 {
        session.pending_cast = Some(Cast {
            skill: id,
            target,
            expires_at: now + spell.cast_ms + 3000,
        });
        super::send_response(sock, peer, &None, payload, "SKILL_CASTING").await;
        return true;
    }
    if payload[1] == 4 {
        if session
            .pending_cast
            .as_ref()
            .is_some_and(|pending| pending.skill == id)
        {
            session.pending_cast = None;
        }
        return true;
    }
    if payload[1] == 2
        && spell.types[0] == 3
        && session
            .pending_cast
            .as_ref()
            .is_some_and(|pending| pending.matches(id, target, now))
    {
        super::send_response(sock, peer, &None, payload, "SKILL_FLYING").await;
        return true;
    }
    if payload[1] != 3 {
        refused(
            sock,
            peer,
            session,
            id,
            "This skill phase is not implemented yet.",
        )
        .await;
        return true;
    }
    if spell.cast_ms > 0
        && spell.skill_group != 0
        && !session
            .skills
            .buffs
            .get(&23)
            .is_some_and(|buff| now < buff.until)
        && !session
            .pending_cast
            .as_ref()
            .is_some_and(|pending| pending.matches(id, target, now))
    {
        refused(
            sock,
            peer,
            session,
            id,
            "No matching active cast was found.",
        )
        .await;
        return true;
    }
    // TODO(re): Validate animation completion against original class/weapon clips.
    session.pending_cast = None;
    if let Some(result) =
        crate::mage_special::handle(sock, peer, lobby, account, session, spell, payload).await
    {
        if let Err(reason) = result {
            refused(sock, peer, session, id, reason).await;
        }
        return true;
    }
    if let Some(result) =
        crate::priest::handle(sock, peer, lobby, account, session, spell, payload).await
    {
        if let Err(reason) = result {
            refused(sock, peer, session, id, reason).await;
        }
        return true;
    }
    if let Some(result) = utility::handle(sock, peer, lobby, account, session, spell, target).await
    {
        if let Err(reason) = result {
            refused(sock, peer, session, id, reason).await;
        }
        return true;
    }
    if catalog.speeds.contains_key(&id) {
        return false;
    }
    let effect = catalog.effects.get(&(spell.types[0], id));
    let stealth = spell.types == [9, 0]
        && effect.is_some_and(|row| {
            matches!(
                (combat::integer(row, 6), combat::integer(row, 7)),
                (1, 1 | 2) | (2, 3 | 4)
            )
        });
    if stealth {
        if target != caster || !matches!(spell.moral, 1 | 2) {
            refused(
                sock,
                peer,
                session,
                id,
                "This stealth effect must target yourself.",
            )
            .await;
            return true;
        }
        let duration = combat::integer(effect.unwrap(), 10).max(0) as u64 * 1000;
        let category = if combat::integer(effect.unwrap(), 6) == 1 {
            91
        } else {
            92
        };
        let result = {
            let db = lobby.db.lock().unwrap();
            skills::load(&db, account, session.character.slot)
                .map_err(|_| "Unable to load stealth state.")
                .and_then(|mut next| {
                    if next
                        .buffs
                        .get(&category)
                        .is_some_and(|buff| now < buff.until)
                    {
                        return Err("This visibility skill category is already active.");
                    }
                    next.spend(spell, now)?;
                    next.buffs.insert(
                        category,
                        skills::Buff {
                            skill: id,
                            until: now + duration,
                        },
                    );
                    skills::save(&db, account, session.character.slot, &next)
                        .map_err(|_| "Unable to save stealth state.")?;
                    Ok(next)
                })
        };
        match result {
            Ok(next) => {
                session.skills = next;
                super::send_response(
                    sock,
                    peer,
                    &None,
                    &session.skills.mana_packet(),
                    "STEALTH_MANA",
                )
                .await;
                super::send_response(
                    sock,
                    peer,
                    &None,
                    &skills::effect(id, caster),
                    "STEALTH_EFFECT",
                )
                .await;
                if category == 91 {
                    visibility(lobby, session, true);
                }
                tracing::info!(%peer,id,duration,category,"visibility skill applied");
            }
            Err(reason) => refused(sock, peer, session, id, reason).await,
        }
        return true;
    }
    let drain = spell.types == [3, 0]
        && effect.is_some_and(|row| {
            combat::integer(row, 2) == 9
                && (1..100).contains(&combat::integer(row, 3))
                && combat::integer(row, 5) == 0
        });
    let secondary = catalog.effects.get(&(spell.types[1], id));
    let npc_secondary = spell.types[0] == 1
        && ((spell.types[1] == 4
            && secondary.is_some_and(|row| matches!(combat::integer(row, 1), 6 | 21)))
            || (matches!(session.character.class % 100, 1 | 5 | 6)
                && spell.types[1] == 3
                && secondary.is_some_and(|row| matches!(combat::integer(row, 2), 0 | 2 | 13)))
            || (spell.types[1] == 9 && secondary.is_some_and(|row| combat::integer(row, 6) == 1)));
    if (spell.types[0] != 1 || (spell.types[1] != 0 && !npc_secondary)) && !drain {
        refused(
            sock,
            peer,
            session,
            id,
            "This skill effect is not implemented yet; no mana was spent.",
        )
        .await;
        return true;
    }
    if spell.moral != 7 {
        refused(
            sock,
            peer,
            session,
            id,
            "This skill's target rules are not implemented yet.",
        )
        .await;
        return true;
    }
    let (Some(monsters), Some(effect)) = (lobby.monsters.as_ref(), effect) else {
        refused(
            sock,
            peer,
            session,
            id,
            "The skill or combat target data is unavailable.",
        )
        .await;
        return true;
    };
    let attack = monsters::Attack {
        position: session.position,
        range: 2.0
            + if spell.range > 0 {
                f32::from(spell.range)
            } else {
                session.combat.range
            },
        attack: session.combat.attack,
        percent: combat::integer(effect, 3).clamp(0, 32767) as u16,
        extra: combat::integer(effect, 4).clamp(0, 32767) as u16,
        skill: true,
        drain_percent: if drain {
            combat::integer(effect, 3) as u8
        } else {
            0
        },
        arrow: false,
        penetrate: false,
        magic: if spell.types[1] == 3
            && secondary.is_some_and(|r| combat::integer(r, 2) == 2 && combat::integer(r, 5) > 0)
        {
            let row = secondary.unwrap();
            Some(monsters::MagicDamage {
                element: combat::integer(row, 6).clamp(0, 6) as u8,
                first: 0,
                over_time: combat::integer(row, 4).unsigned_abs().min(32767) as u16,
                duration: combat::integer(row, 5).clamp(0, 255) as u8,
                charisma: 60,
                bonus: 0,
                raw_over_time: true,
                raw_first: false,
            })
        } else {
            None
        },
    };
    let mut paid = None;
    let result = monsters.hit_session_owned(target, attack, session, false, || {
        paid = Some(resources::commit(
            lobby,
            account,
            session.character.slot,
            spell,
            1,
            |_| Ok(()),
        )?);
        Ok(())
    });
    match result {
        Ok(hit) => {
            resources::sync(sock, peer, lobby, account, session, paid.unwrap()).await;
            if !drain {
                crate::durability::attack(
                    sock,
                    peer,
                    lobby,
                    account,
                    session,
                    hit.damage.min(32767) as u16,
                )
                .await;
            }
            if spell.types[1] == 4
                && let Some(row) = secondary
                && combat::integer(row, 1) == 6
            {
                monsters.slow(
                    target,
                    combat::integer(row, 5).clamp(1, 200) as u16,
                    combat::integer(row, 3).max(0) as u64 * 1000,
                );
            }
            match remove_stealth(lobby, account, session, false) {
                Ok(true) => {
                    super::send_response(sock, peer, &None, &[0x31, 5, 91], "STEALTH_BROKEN").await
                }
                Err(error) => tracing::error!(%peer,%error,"stealth break failed"),
                _ => {}
            }
            let mut packet = skills::effect(id, caster);
            packet[8..10].copy_from_slice(&target.to_le_bytes());
            super::send_response(sock, peer, &None, &packet, "SKILL_EFFECT").await;
            if drain && hit.damage > 32767 {
                let text = format!("{} inflicted {} damage.", spell.name, hit.damage);
                let notice = crate::chat::packet(
                    7,
                    session.character.nation(),
                    u16::MAX,
                    "",
                    text.as_bytes(),
                );
                super::send_response(sock, peer, &None, &notice, "PERCENT_DAMAGE_TOTAL").await;
            }
            tracing::info!(%peer,id,name=%spell.name,target,damage=hit.damage,dead=hit.dead,drain,"combat skill applied");
        }
        Err(reason) => refused(sock, peer, session, id, reason).await,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use std::sync::Arc;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn rogue_completion_applies_drains_immediately_and_preserves_stealth_rules() {
        let game = reko_common::game_dir();
        if !game.join("Data/Skill_Magic_Main_us.tbl").exists() {
            return;
        }
        let mut lobby = Lobby::new(
            Connection::open_in_memory().unwrap(),
            crate::characters::CreationRules::load(&game).unwrap(),
            crate::world_entry::Bootstrap { next_level_exp: 50 },
        )
        .unwrap();
        lobby.skills = Some(skills::Catalog::load(&game).unwrap());
        lobby.monsters = Some(Arc::new(monsters::World::load(&game).unwrap()));
        let character = crate::characters::Character {
            slot: 0,
            name: "RogueSkills".into(),
            race: 2,
            class: 108,
            face: 0,
            hair: 0,
            stats: [60, 150, 255, 50, 50],
        };
        {
            let mut db = lobby.db.lock().unwrap();
            crate::characters::ensure_account(&db, "admin").unwrap();
            crate::characters::set_nation(&db, "admin", 1).unwrap();
            crate::characters::create(&mut db, "admin", &character).unwrap();
        }
        let mut session = Session::new(character).unwrap();
        session.entered = true;
        session.position.x = 3250;
        session.position.z = 4000;
        session.progress.level = 80;
        session.progress.points = [0, 0, 0, 0, 0, 0, 80, 80, 0];
        session.combat.weapon_kind = 11;
        session.base_combat = session.combat;
        session.skills.set_limits(2690, 4110);
        skills::save(&lobby.db.lock().unwrap(), "admin", 0, &session.skills).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let _client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (mut socket, peer) = listener.accept().await.unwrap();
        let request = |id, target, stage| {
            let mut p = skills::effect(id, session.player_id);
            p[1] = stage;
            p[8..10].copy_from_slice(&u16::to_le_bytes(target));
            p
        };
        let vamp_cast = request(108650, 10000, 1);
        let vamp_fly = request(108650, 10000, 2);
        let vamp_hit = request(108650, 10000, 3);
        let stealth_cast = request(108645, session.player_id, 1);
        let stealth_hit = request(108645, session.player_id, 3);
        let hide_cast = request(108700, session.player_id, 1);
        let hide_hit = request(108700, session.player_id, 3);
        let blood_cast = request(108610, 10000, 1);
        let blood_hit = request(108610, 10000, 3);
        for payload in [&vamp_cast, &vamp_fly, &vamp_hit] {
            assert!(
                handle(
                    &mut socket,
                    peer,
                    payload,
                    &lobby,
                    Some("admin"),
                    Some(&mut session)
                )
                .await
            );
        }
        assert!(session.pending_cast.is_none());
        assert_eq!(session.skills.mana, 4060);
        assert_eq!(session.skills.hp, 2690);
        let health = lobby.monsters.as_ref().unwrap().health(10000, 21).unwrap();
        assert_eq!(u32::from_le_bytes(health[8..12].try_into().unwrap()), 22500);
        let mana = session.skills.mana;
        handle(
            &mut socket,
            peer,
            &vamp_hit,
            &lobby,
            Some("admin"),
            Some(&mut session),
        )
        .await;
        assert_eq!(session.skills.mana, mana);
        handle(
            &mut socket,
            peer,
            &blood_cast,
            &lobby,
            Some("admin"),
            Some(&mut session),
        )
        .await;
        handle(
            &mut socket,
            peer,
            &blood_hit,
            &lobby,
            Some("admin"),
            Some(&mut session),
        )
        .await;
        let health = lobby.monsters.as_ref().unwrap().health(10000, 21).unwrap();
        assert_eq!(u32::from_le_bytes(health[8..12].try_into().unwrap()), 21375);
        session.skills.buffs.insert(
            6,
            skills::Buff {
                skill: 108010,
                until: skills::now_ms() + 600000,
            },
        );
        skills::save(&lobby.db.lock().unwrap(), "admin", 0, &session.skills).unwrap();
        let mut events = lobby.monsters.as_ref().unwrap().events.subscribe();
        handle(
            &mut socket,
            peer,
            &stealth_cast,
            &lobby,
            Some("admin"),
            Some(&mut session),
        )
        .await;
        handle(
            &mut socket,
            peer,
            &stealth_hit,
            &lobby,
            Some("admin"),
            Some(&mut session),
        )
        .await;
        assert_eq!(session.skills.buffs[&91].skill, 108645);
        let event = events.try_recv().unwrap();
        let mut expected = vec![0x29];
        expected.extend(session.player_id.to_le_bytes());
        expected.extend([7, 1, 0, 0, 0]);
        assert_eq!(event.packet, expected);
        assert!(!remove_stealth(&lobby, "admin", &mut session, true).unwrap());
        assert!(remove_stealth(&lobby, "admin", &mut session, false).unwrap());
        assert!(
            !skills::load(&lobby.db.lock().unwrap(), "admin", 0)
                .unwrap()
                .buffs
                .contains_key(&91)
        );
        handle(
            &mut socket,
            peer,
            &hide_cast,
            &lobby,
            Some("admin"),
            Some(&mut session),
        )
        .await;
        handle(
            &mut socket,
            peer,
            &hide_hit,
            &lobby,
            Some("admin"),
            Some(&mut session),
        )
        .await;
        assert_eq!(session.skills.buffs[&91].skill, 108700);
        assert!(remove_stealth(&lobby, "admin", &mut session, true).unwrap());
        assert_eq!(session.skills.buffs[&6].skill, 108010);
        for (target, x, z, id) in [
            (10000, 3205, 4000, 108615),
            (10001, 3235, 4030, 108655),
            (10002, 3205, 4060, 108640),
        ] {
            session.position.x = x;
            session.position.z = z;
            let mut packet = skills::effect(id, session.player_id);
            packet[8..10].copy_from_slice(&u16::to_le_bytes(target));
            let mana = session.skills.mana;
            handle(
                &mut socket,
                peer,
                &packet,
                &lobby,
                Some("admin"),
                Some(&mut session),
            )
            .await;
            assert!(
                session.skills.mana < mana,
                "stationary 4.5m skill rejected for target {target}"
            );
        }
    }
}
