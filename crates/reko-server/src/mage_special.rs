//! Source-based Mage transport and bounded player resource/control effects.
use crate::{
    combat, lobby::Lobby, movement::Position, resources, skills, social, world_entry::Session,
};
use std::net::SocketAddr;
use tokio::net::TcpStream;

fn active(s: &Session, category: u8) -> bool {
    s.skills
        .buffs
        .get(&category)
        .is_some_and(|b| skills::now_ms() < b.until)
}

fn enemy(caster: &Session, target: &Session) -> bool {
    caster.player_id != target.player_id
        && caster.position.zone == target.position.zone
        && target.entered
        && !target.zone_loading
        && target.skills.hp > 0
        && !target.is_admin
        && (caster.position.zone == 48
            || matches!(caster.position.zone, 71 | 72)
                && caster.character.nation() != target.character.nation())
}

fn effect(
    spell: &skills::Spell,
    caster: u16,
    target: u16,
    row: Option<&[reko_assets::CellValue]>,
) -> Vec<u8> {
    let mut packet = skills::effect(spell.id, caster);
    packet[8..10].copy_from_slice(&target.to_le_bytes());
    packet[12..14].copy_from_slice(&1u16.to_le_bytes());
    if let Some(row) = row {
        packet[16..18]
            .copy_from_slice(&(combat::integer(row, 3).clamp(0, 32767) as u16).to_le_bytes());
        packet[20..22]
            .copy_from_slice(&(combat::integer(row, 5).clamp(0, 200) as u16).to_le_bytes());
    }
    packet
}

fn notify(hub: &mut social::Hub, player: &social::Player, state: &skills::State, packet: Vec<u8>) {
    let id = player.s.player_id;
    hub.skill_refresh.insert(id);
    if let Some(p) = hub.players.get_mut(&id) {
        p.s.skills = state.clone();
    }
    hub.send(id, packet);
}

fn transport(
    lobby: &Lobby,
    account: &str,
    s: &Session,
    spell: &skills::Spell,
    target: u16,
) -> Result<(resources::Paid, Option<Position>), &'static str> {
    let db = lobby.db.lock().unwrap();
    let data: String = db
        .query_row(
            "SELECT data FROM world_reference_rules WHERE table_name='MAGIC_TYPE8' AND rule_id=?1",
            [spell.id],
            |r| r.get(0),
        )
        .map_err(|_| "Original 1453 transport rules have not been imported.")?;
    let rule: serde_json::Value =
        serde_json::from_str(&data).map_err(|_| "Invalid transport rule.")?;
    let value = |key: &str| {
        rule[key]
            .as_str()
            .and_then(|v| v.parse::<u16>().ok())
            .or_else(|| rule[key].as_u64().and_then(|v| v.try_into().ok()))
            .unwrap_or(0)
    };
    let kind = value("WarpType");
    let radius = value("Radius");
    if !matches!(kind, 1 | 12 | 20) || radius == 0 {
        return Err("Unsupported transport rule.");
    }
    if active(s, 22) || s.position.zone == 55 || spell.moral == 6 && s.position.zone > 31 {
        return Err("Teleport is unavailable in this state or zone.");
    }
    let mut hub = lobby.social.lock().unwrap();
    let group = hub.group(s.player_id);
    let mut recipients = Vec::new();
    if kind == 12 {
        if target == s.player_id || !group.contains(&target) {
            return Err("Select a living party member in this zone.");
        }
        let player = hub
            .players
            .get(&target)
            .ok_or("The selected party member is offline.")?
            .clone();
        if !player.s.entered
            || player.s.zone_loading
            || player.s.skills.hp == 0
            || active(&player.s, 22)
            || !social::near(s.position, player.s.position, radius)
            || hub.transfers.contains_key(&target)
        {
            return Err("The selected party member cannot be summoned.");
        }
        recipients.push((
            player,
            s.movement
                .current_position(s.position, std::time::Instant::now()),
        ));
    } else {
        if target != s.player_id && !(target == u16::MAX && spell.moral == 6) {
            return Err("Invalid teleport target.");
        }
        let destination = if kind == 20 {
            let position = s
                .movement
                .forward(
                    s.movement
                        .current_position(s.position, std::time::Instant::now()),
                    radius,
                )
                .ok_or("Move in the desired direction before using Blink.")?;
            let bounds: String = db.query_row("SELECT data FROM world_reference_rules WHERE table_name='MAP_BOUNDS' AND rule_id=?1",[s.position.zone],|r|r.get(0))
                .map_err(|_| "Original map bounds have not been imported.")?;
            let bounds: serde_json::Value =
                serde_json::from_str(&bounds).map_err(|_| "Invalid map bounds.")?;
            let extent = bounds["extent"]
                .as_u64()
                .filter(|v| *v > 0 && *v <= 65535)
                .ok_or("Invalid map bounds.")?;
            if u64::from(position.x) >= extent || u64::from(position.z) >= extent {
                return Err("Blink would leave the map.");
            }
            position
        } else {
            crate::town::start_position(&db, s)?
        };
        recipients.push((
            social::Player {
                account: account.into(),
                s: s.clone(),
            },
            destination,
        ));
        if spell.moral == 6 {
            for id in group {
                if id == s.player_id || hub.transfers.contains_key(&id) {
                    continue;
                }
                let Some(player) = hub.players.get(&id) else {
                    continue;
                };
                if player.s.entered
                    && !player.s.zone_loading
                    && player.s.skills.hp > 0
                    && !active(&player.s, 22)
                    && social::near(s.position, player.s.position, radius)
                {
                    recipients.push((player.clone(), crate::town::start_position(&db, &player.s)?));
                }
            }
        }
    }
    let tx = db
        .unchecked_transaction()
        .map_err(|_| "Unable to start teleport transaction.")?;
    let paid = resources::commit_in(&tx, account, s.character.slot, spell, 1, |_| Ok(()))?;
    for (player, destination) in &recipients {
        crate::movement::save(&tx, &player.account, player.s.character.slot, *destination)
            .map_err(|_| "Unable to save teleport destination.")?;
    }
    tx.commit().map_err(|_| "Unable to commit teleport.")?;
    let mut own = None;
    for (player, destination) in recipients {
        if player.s.player_id == s.player_id {
            own = Some(destination)
        } else {
            hub.transfers.insert(player.s.player_id, destination);
            hub.send(
                player.s.player_id,
                effect(spell, s.player_id, player.s.player_id, None),
            );
        }
    }
    Ok((paid, own))
}

fn player_effect(
    lobby: &Lobby,
    account: &str,
    s: &Session,
    spell: &skills::Spell,
    payload: &[u8],
    category: u8,
) -> Result<resources::Paid, &'static str> {
    let target = u16::from_le_bytes([payload[8], payload[9]]);
    let catalog = lobby.skills.as_ref().ok_or("Skill data unavailable.")?;
    let drain = spell.types == [3, 0];
    let row = &catalog.effects[&(if drain { 3 } else { 4 }, spell.id)];
    let friendly = matches!(category, 2 | 8);
    let center = if spell.moral == 10 {
        if target != u16::MAX {
            return Err("Invalid area target.");
        }
        let x = i16::from_le_bytes([payload[10], payload[11]]);
        let z = i16::from_le_bytes([payload[14], payload[15]]);
        let center = Position {
            x: u16::try_from(i32::from(x) * 10).map_err(|_| "Invalid area coordinates.")?,
            z: u16::try_from(i32::from(z) * 10).map_err(|_| "Invalid area coordinates.")?,
            ..s.position
        };
        if !social::near(s.position, center, spell.range) {
            return Err("The ground target is too far away.");
        }
        center
    } else {
        s.position
    };
    let db = lobby.db.lock().unwrap();
    let mut hub = lobby.social.lock().unwrap();
    let radius = if spell.moral == 10 {
        combat::integer(row, 2).clamp(0, 10000) as u16
    } else {
        spell.range
    };
    let players: Vec<_> = hub
        .players
        .values()
        .filter(|p| {
            (spell.moral == 10 || p.s.player_id == target)
                && social::near(center, p.s.position, radius)
                && if friendly {
                    p.s.player_id != s.player_id
                        && p.s.entered
                        && !p.s.zone_loading
                        && p.s.skills.hp > 0
                        && p.s.character.nation() == s.character.nation()
                        && (spell.moral != 4
                            || hub.party(s.player_id).is_some()
                                && hub.party(s.player_id) == hub.party(p.s.player_id))
                } else {
                    enemy(s, &p.s)
                        && hub
                            .party(s.player_id)
                            .is_none_or(|party| hub.party(p.s.player_id) != Some(party))
                }
        })
        .cloned()
        .collect();
    if players.is_empty() {
        return Err(if friendly {
            "Select a living friendly player within range."
        } else {
            "This spell requires an enemy player in a combat zone; it does not affect monsters."
        });
    }
    let tx = db
        .unchecked_transaction()
        .map_err(|_| "Unable to start player skill transaction.")?;
    let mut paid = resources::commit_in(&tx, account, s.character.slot, spell, 1, |_| Ok(()))?;
    let mut changes = Vec::new();
    for player in players {
        let mut state = skills::load(&tx, &player.account, player.s.character.slot)
            .map_err(|_| "Unable to load target resources.")?;
        if state.hp == 0 {
            return Err("The target is dead.");
        }
        if drain {
            let amount = combat::integer(row, 3).min(0).unsigned_abs().min(32767) as u16;
            state.mana = state.mana.saturating_sub(amount);
            paid.state.hp = paid
                .state
                .hp
                .saturating_add(amount / 2)
                .min(paid.state.max_hp);
        } else {
            if state
                .buffs
                .get(&category)
                .is_some_and(|b| skills::now_ms() < b.until)
            {
                return Err("An effect of this category is already active.");
            }
            state.buffs.insert(
                category,
                skills::Buff {
                    skill: spell.id,
                    until: skills::now_ms() + combat::integer(row, 3).max(0) as u64 * 1000,
                },
            );
        }
        skills::save_in(&tx, &player.account, player.s.character.slot, &state)
            .map_err(|_| "Unable to save target effect.")?;
        changes.push((player, state));
    }
    skills::save_in(&tx, account, s.character.slot, &paid.state)
        .map_err(|_| "Unable to save caster resources.")?;
    tx.commit().map_err(|_| "Unable to commit player skill.")?;
    for (player, state) in changes {
        let packet = effect(
            spell,
            s.player_id,
            player.s.player_id,
            (!drain).then_some(row.as_slice()),
        );
        let mut packet = packet;
        packet[10..12].copy_from_slice(&payload[10..12]);
        packet[14..16].copy_from_slice(&payload[14..16]);
        notify(&mut hub, &player, &state, packet);
    }
    Ok(paid)
}

pub(crate) async fn handle(
    sock: &mut TcpStream,
    peer: SocketAddr,
    lobby: &Lobby,
    account: &str,
    s: &mut Session,
    spell: &skills::Spell,
    payload: &[u8],
) -> Option<Result<(), &'static str>> {
    if !matches!(s.character.class % 100, 3 | 9 | 10) {
        return None;
    }
    let catalog = lobby.skills.as_ref()?;
    let target = u16::from_le_bytes([payload[8], payload[9]]);
    let category = catalog
        .effects
        .get(&(4, spell.id))
        .map_or(0, |row| combat::integer(row, 1) as u8);
    let drain = spell.types == [3, 0]
        && catalog
            .effects
            .get(&(3, spell.id))
            .is_some_and(|r| combat::integer(r, 2) == 16);
    if spell.types == [8, 0] {
        return Some(match transport(lobby, account, s, spell, target) {
            Err(reason) => Err(reason),
            Ok((paid, destination)) => {
                resources::sync(sock, peer, lobby, account, s, paid).await;
                super::send_response(
                    sock,
                    peer,
                    &None,
                    &effect(spell, s.player_id, target, None),
                    "MAGE_TRANSPORT_EFFECT",
                )
                .await;
                if let Some(position) = destination {
                    let packet = crate::travel::begin(lobby, s, position);
                    super::send_response(sock, peer, &None, &packet, "MAGE_TRANSPORT").await;
                    crate::population::refresh(sock, peer, lobby, s).await;
                }
                Ok(())
            }
        });
    }
    if !drain
        && !(spell.types == [4, 0]
            && (matches!(category, 20 | 22 | 24)
                || matches!(category, 2 | 8) && target != s.player_id))
    {
        return None;
    }
    Some(
        match player_effect(lobby, account, s, spell, payload, category) {
            Err(reason) => Err(reason),
            Ok(paid) => {
                resources::sync(sock, peer, lobby, account, s, paid).await;
                let mut packet = payload.to_vec();
                packet[12..14].copy_from_slice(&1u16.to_le_bytes());
                super::send_response(sock, peer, &None, &packet, "MAGE_PLAYER_EFFECT").await;
                Ok(())
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn player_control_blocks_movement_and_targeted_attacks_until_expiry() {
        let Some((mut lobby, mut s)) = setup(1) else {
            return;
        };
        lobby.monsters = Some(std::sync::Arc::new(
            crate::monsters::World::load(&reko_common::game_dir()).unwrap(),
        ));
        s.base_combat.attack = 1000;
        s.base_combat.range = 3.0;
        s.base_combat.weapon_kind = 110;
        s.combat = s.base_combat;
        for (category, skill) in [(20, 110762), (22, 110674)] {
            s.skills.buffs.insert(
                category,
                skills::Buff {
                    skill,
                    until: skills::now_ms() + 15000,
                },
            );
        }
        skills::save(&lobby.db.lock().unwrap(), "MageFixture", 0, &s.skills).unwrap();
        let position = s.position;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let _client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (mut sock, peer) = listener.accept().await.unwrap();
        let mut world = Some(s);
        let mut account = Some("MageFixture".to_owned());
        let mut movement = vec![6];
        movement.extend((position.x + 40).to_le_bytes());
        movement.extend(position.z.to_le_bytes());
        movement.extend(0i16.to_le_bytes());
        movement.extend(45i16.to_le_bytes());
        movement.push(0);
        crate::handle_game_payload(&mut sock, peer, &movement, &lobby, &mut account, &mut world)
            .await;
        assert_eq!(world.as_ref().unwrap().position, position);
        let hp = || {
            let p = lobby.monsters.as_ref().unwrap().health(10000, 21).unwrap();
            u32::from_le_bytes(p[8..12].try_into().unwrap())
        };
        let before = hp();
        let mut attack = vec![8, 1, 0];
        attack.extend(10000u16.to_le_bytes());
        attack.extend([0; 4]);
        crate::handle_game_payload(&mut sock, peer, &attack, &lobby, &mut account, &mut world)
            .await;
        assert_eq!(hp(), before);
        assert_eq!(world.as_ref().unwrap().next_attack, 0);
        let p = packet(world.as_ref().unwrap(), 110515, 10000, 1);
        crate::handle_game_payload(&mut sock, peer, &p, &lobby, &mut account, &mut world).await;
        assert!(world.as_ref().unwrap().pending_cast.is_none());
        let s = world.as_ref().unwrap();
        let cancel = packet(s, 110674, s.player_id, 6);
        crate::handle_game_payload(&mut sock, peer, &cancel, &lobby, &mut account, &mut world)
            .await;
        assert!(world.as_ref().unwrap().skills.buffs.contains_key(&22));
        let s = world.as_mut().unwrap();
        assert_eq!(s.skills.mana, 10000);
        s.skills.expire(skills::now_ms() + 20000);
        skills::save(&lobby.db.lock().unwrap(), "MageFixture", 0, &s.skills).unwrap();
        crate::handle_game_payload(&mut sock, peer, &attack, &lobby, &mut account, &mut world)
            .await;
        assert!(hp() < before);
        crate::handle_game_payload(&mut sock, peer, &movement, &lobby, &mut account, &mut world)
            .await;
        assert_eq!(world.as_ref().unwrap().position.x, position.x + 40);
    }

    #[tokio::test]
    async fn blink_uses_only_validated_movement_heading_and_ignores_client_destination() {
        for nation in [1, 2] {
            let Some((lobby, mut s)) = setup(nation) else {
                return;
            };
            let id = u32::from(s.character.class) * 1000 + 774;
            let spell = &lobby.skills.as_ref().unwrap().spells[&id];
            assert!(transport(&lobby, "MageFixture", &s, spell, s.player_id).is_err());
            let now = std::time::Instant::now();
            let mut move_packet = vec![6];
            move_packet.extend(65000u16.to_le_bytes());
            move_packet.extend(s.position.z.to_le_bytes());
            move_packet.extend(0i16.to_le_bytes());
            move_packet.extend(45i16.to_le_bytes());
            move_packet.push(1);
            assert!(
                s.movement
                    .validate(&move_packet, s.position, now, 45)
                    .is_none()
            );
            assert!(s.movement.forward(s.position, 20).is_none());
            move_packet[1..3].copy_from_slice(&(s.position.x + 45).to_le_bytes());
            s.position = s
                .movement
                .validate(&move_packet, s.position, now, 45)
                .unwrap();
            let mut boundary = s.clone();
            boundary.position.x = 5110;
            assert!(
                transport(&lobby, "MageFixture", &boundary, spell, boundary.player_id).is_err()
            );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let _client = TcpStream::connect(listener.local_addr().unwrap())
                .await
                .unwrap();
            let (mut sock, peer) = listener.accept().await.unwrap();
            for phase in [1, 3] {
                let mut p = packet(&s, id, s.player_id, phase);
                p[10..12].copy_from_slice(&6000u16.to_le_bytes());
                p[14..16].copy_from_slice(&6000u16.to_le_bytes());
                crate::magic::handle(
                    &mut sock,
                    peer,
                    &p,
                    &lobby,
                    Some("MageFixture"),
                    Some(&mut s),
                )
                .await;
            }
            assert!((3450..=3495).contains(&s.position.x));
            assert_eq!(s.position.z, 4000);
            assert_eq!(s.skills.mana, 10000 - spell.mana);
            assert_eq!(
                crate::movement::load(&lobby.db.lock().unwrap(), "MageFixture", 0).unwrap(),
                s.position
            );
            assert!(s.entered && !s.zone_loading);
        }
    }

    fn setup(nation: u8) -> Option<(Lobby, Session)> {
        let game = reko_common::game_dir();
        if !game.join("Data/Skill_Magic_Main_us.tbl").exists() {
            return None;
        }
        let mut lobby = social::tests::lobby();
        lobby.skills = Some(skills::Catalog::load(&game).unwrap());
        let mut s = social::tests::player(&lobby, "MageFixture", nation, 80);
        s.character.class = if nation == 1 { 110 } else { 210 };
        s.position = Position {
            zone: 21,
            x: 3250,
            z: 4000,
            y: 0,
        };
        s.progress.points = [0, 0, 0, 0, 0, 80, 80, 80, 20];
        s.skills.set_limits(5000, 10000);
        let db = lobby.db.lock().unwrap();
        db.execute(
            "UPDATE lobby_characters SET class=?1 WHERE account='MageFixture'",
            [s.character.class],
        )
        .unwrap();
        skills::save(&db, "MageFixture", 0, &s.skills).unwrap();
        for quest in [53, 515, 516, 517] {
            db.execute(
                "INSERT INTO character_quests VALUES('MageFixture',0,?1,2)",
                [quest],
            )
            .unwrap();
        }
        db.execute(
            "INSERT INTO character_inventory VALUES('MageFixture',0,20,379061000,1,20)",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO world_town_starts VALUES(21,306,352,306,352,0,0,'fixture')",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO world_reference_rules VALUES('MAP_BOUNDS',21,'{\"extent\":5120}')",
            [],
        )
        .unwrap();
        for (suffix, kind, radius) in [
            (4, 12, 10000),
            (15, 1, 10000),
            (35, 1, 10000),
            (774, 20, 20),
        ] {
            let id = u32::from(s.character.class) * 1000 + suffix;
            let row = serde_json::json!({"WarpType":kind,"Radius":radius});
            db.execute(
                "INSERT INTO world_reference_rules VALUES('MAGIC_TYPE8',?1,?2)",
                rusqlite::params![id, row.to_string()],
            )
            .unwrap();
        }
        drop(db);
        social::synchronize(&lobby, "MageFixture", &mut s);
        Some((lobby, s))
    }

    fn packet(s: &Session, id: u32, target: u16, phase: u8) -> Vec<u8> {
        let mut p = skills::effect(id, s.player_id);
        p[1] = phase;
        p[8..10].copy_from_slice(&target.to_le_bytes());
        if target == u16::MAX {
            p[10..12].copy_from_slice(&(s.position.x / 10).to_le_bytes());
            p[14..16].copy_from_slice(&(s.position.z / 10).to_le_bytes());
        }
        p
    }

    #[test]
    fn mage_transport_saves_party_positions_and_payment_atomically_and_rejects_bad_targets() {
        for nation in [1, 2] {
            let Some((lobby, mut s)) = setup(nation) else {
                return;
            };
            let mut member = social::tests::player(&lobby, "MageParty", nation, 50);
            member.position = s.position;
            member.skills.set_limits(1000, 1000);
            skills::save(&lobby.db.lock().unwrap(), "MageParty", 0, &member.skills).unwrap();
            social::synchronize(&lobby, "MageParty", &mut member);
            let prefix = u32::from(s.character.class) * 1000;
            let catalog = lobby.skills.as_ref().unwrap();
            let summon = &catalog.spells[&(prefix + 4)];
            assert!(transport(&lobby, "MageFixture", &s, summon, member.player_id).is_err());
            assert_eq!(
                skills::load(&lobby.db.lock().unwrap(), "MageFixture", 0)
                    .unwrap()
                    .mana,
                10000
            );
            lobby
                .social
                .lock()
                .unwrap()
                .parties
                .insert(s.player_id, vec![s.player_id, member.player_id]);
            let (paid, own) =
                transport(&lobby, "MageFixture", &s, summon, member.player_id).unwrap();
            assert!(own.is_none());
            assert_eq!(paid.state.mana, 10000 - summon.mana);
            assert_eq!(
                crate::movement::load(&lobby.db.lock().unwrap(), "MageParty", 0).unwrap(),
                s.position
            );
            assert!(transport(&lobby, "MageFixture", &s, summon, member.player_id).is_err());
            assert_eq!(
                lobby
                    .social
                    .lock()
                    .unwrap()
                    .transfers
                    .remove(&member.player_id),
                Some(s.position)
            );
            let escape = &catalog.spells[&(prefix + 35)];
            let (paid, own) = transport(&lobby, "MageFixture", &s, escape, u16::MAX).unwrap();
            let own = own.unwrap();
            assert_eq!((own.x, own.z), (3060, 3520));
            assert_eq!(
                crate::movement::load(&lobby.db.lock().unwrap(), "MageParty", 0).unwrap(),
                own
            );
            assert_eq!(paid.state.mana, 10000 - summon.mana - escape.mana);
            let mut poor = skills::load(&lobby.db.lock().unwrap(), "MageFixture", 0).unwrap();
            poor.mana = 0;
            skills::save(&lobby.db.lock().unwrap(), "MageFixture", 0, &poor).unwrap();
            let before =
                crate::movement::load(&lobby.db.lock().unwrap(), "MageFixture", 0).unwrap();
            assert!(
                transport(
                    &lobby,
                    "MageFixture",
                    &s,
                    &catalog.spells[&(prefix + 15)],
                    s.player_id
                )
                .is_err()
            );
            assert_eq!(
                crate::movement::load(&lobby.db.lock().unwrap(), "MageFixture", 0).unwrap(),
                before
            );
            s.skills.buffs.insert(
                22,
                skills::Buff {
                    skill: prefix + 674,
                    until: skills::now_ms() + 15000,
                },
            );
            assert!(
                transport(
                    &lobby,
                    "MageFixture",
                    &s,
                    &catalog.spells[&(prefix + 774)],
                    s.player_id
                )
                .is_err()
            );
        }
    }

    #[test]
    fn player_only_mage_effects_preserve_npc_rules_and_commit_both_players() {
        for nation in [1, 2] {
            let Some((lobby, mut s)) = setup(nation) else {
                return;
            };
            let mut target = social::tests::player(&lobby, "EnemyFixture", 3 - nation, 80);
            s.position.zone = 71;
            target.position = s.position;
            s.skills.hp = 1000;
            target.skills.set_limits(5000, 5000);
            skills::save(&lobby.db.lock().unwrap(), "MageFixture", 0, &s.skills).unwrap();
            skills::save(&lobby.db.lock().unwrap(), "EnemyFixture", 0, &target.skills).unwrap();
            social::synchronize(&lobby, "MageFixture", &mut s);
            social::synchronize(&lobby, "EnemyFixture", &mut target);
            let prefix = u32::from(s.character.class) * 1000;
            let catalog = lobby.skills.as_ref().unwrap();
            let drain = &catalog.spells[&(prefix + 574)];
            assert!(
                player_effect(
                    &lobby,
                    "MageFixture",
                    &s,
                    drain,
                    &packet(&s, drain.id, 10000, 3),
                    0
                )
                .is_err()
            );
            assert_eq!(
                skills::load(&lobby.db.lock().unwrap(), "MageFixture", 0)
                    .unwrap()
                    .mana,
                10000
            );
            let paid = player_effect(
                &lobby,
                "MageFixture",
                &s,
                drain,
                &packet(&s, drain.id, target.player_id, 3),
                0,
            )
            .unwrap();
            assert_eq!(paid.state.hp, 2500);
            assert_eq!(
                skills::load(&lobby.db.lock().unwrap(), "EnemyFixture", 0)
                    .unwrap()
                    .mana,
                2000
            );
            assert!(
                player_effect(
                    &lobby,
                    "MageFixture",
                    &s,
                    drain,
                    &packet(&s, drain.id, target.player_id, 3),
                    0
                )
                .is_err()
            );
            for (suffix, category) in [(674, 22), (762, 20), (825, 24)] {
                let spell = &catalog.spells[&(prefix + suffix)];
                let request = packet(
                    &s,
                    spell.id,
                    if spell.moral == 10 {
                        u16::MAX
                    } else {
                        target.player_id
                    },
                    3,
                );
                player_effect(&lobby, "MageFixture", &s, spell, &request, category).unwrap();
                let mut persisted =
                    skills::load(&lobby.db.lock().unwrap(), "EnemyFixture", 0).unwrap();
                assert_eq!(persisted.buffs[&category].skill, spell.id);
                if category == 22 {
                    assert_eq!(persisted.speed(Some(catalog), skills::now_ms()), 1);
                }
                assert!(
                    persisted
                        .expire(skills::now_ms() + 20000)
                        .contains(&category)
                );
            }
            social::synchronize(&lobby, "EnemyFixture", &mut target);
            assert_eq!(target.skills.mana, 2000);
            assert!(target.skills.buffs.contains_key(&22));
            target.base_combat.resistance = [100; 6];
            let stats = crate::utility::effective(&target, &target.skills, Some(catalog), None);
            assert_eq!(stats.resistance, [80; 6]);
            let mut safe = s.clone();
            safe.position.zone = 21;
            assert!(
                player_effect(
                    &lobby,
                    "MageFixture",
                    &safe,
                    &catalog.spells[&(prefix + 674)],
                    &packet(&safe, prefix + 674, target.player_id, 3),
                    22
                )
                .is_err()
            );
        }
    }

    #[tokio::test]
    async fn original_level80_fire_and_three_armors_execute_through_socket_handler_for_both_nations()
     {
        for nation in [1, 2] {
            let Some((mut lobby, mut s)) = setup(nation) else {
                return;
            };
            lobby.monsters = Some(std::sync::Arc::new(
                crate::monsters::World::load(&reko_common::game_dir()).unwrap(),
            ));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let _client = TcpStream::connect(listener.local_addr().unwrap())
                .await
                .unwrap();
            let (mut sock, peer) = listener.accept().await.unwrap();
            let prefix = u32::from(s.character.class) * 1000;
            let hp = || {
                let p = lobby.monsters.as_ref().unwrap().health(10000, 21).unwrap();
                u32::from_le_bytes(p[8..12].try_into().unwrap())
            };
            let before = hp();
            for phase in [1, 2, 3] {
                let p = packet(&s, prefix + 575, 10000, phase);
                crate::magic::handle(
                    &mut sock,
                    peer,
                    &p,
                    &lobby,
                    Some("MageFixture"),
                    Some(&mut s),
                )
                .await;
            }
            assert!(hp() < before);
            assert_eq!(
                s.skills.mana,
                10000 - lobby.skills.as_ref().unwrap().spells[&(prefix + 575)].mana
            );
            let after = hp();
            let p = packet(&s, prefix + 575, 10000, 3);
            crate::magic::handle(
                &mut sock,
                peer,
                &p,
                &lobby,
                Some("MageFixture"),
                Some(&mut s),
            )
            .await;
            assert_eq!(hp(), after);
            for suffix in [573, 673, 773] {
                s.skills.buffs.remove(&25);
                skills::save(&lobby.db.lock().unwrap(), "MageFixture", 0, &s.skills).unwrap();
                for phase in [1, 3] {
                    let p = packet(&s, prefix + suffix, s.player_id, phase);
                    crate::magic::handle(
                        &mut sock,
                        peer,
                        &p,
                        &lobby,
                        Some("MageFixture"),
                        Some(&mut s),
                    )
                    .await;
                }
                assert_eq!(s.skills.buffs[&25].skill, prefix + suffix);
            }
            let bag =
                crate::starter::inventory(&lobby.db.lock().unwrap(), "MageFixture", 0).unwrap();
            assert_eq!(bag[20].count, 17);
        }
    }
}
