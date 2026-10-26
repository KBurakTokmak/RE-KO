//! Native target bars for visible players, monsters and noncombat NPCs.
use crate::{lobby::Lobby, world_entry::Session};

pub(crate) fn packet(id: u16, maximum: u32, current: u32) -> Vec<u8> {
    let mut packet = vec![0x22];
    packet.extend(id.to_le_bytes());
    packet.push(1);
    packet.extend(maximum.to_le_bytes());
    packet.extend(current.to_le_bytes());
    packet.extend(0i16.to_le_bytes());
    packet
}

pub(crate) fn query(lobby: &Lobby, s: &Session, id: u16) -> Option<Vec<u8>> {
    if !s.entered || s.zone_loading {
        return None;
    }
    if id == s.player_id {
        return Some(packet(
            id,
            u32::from(s.skills.max_hp),
            u32::from(s.skills.hp),
        ));
    }
    if id < 10000 {
        let hub = lobby.social.lock().unwrap();
        let target = &hub.players.get(&id)?.s;
        return (target.entered
            && !target.zone_loading
            && hub.can_see(s.player_id, id)
            && crate::population::same_region_view(s.position, target.position))
        .then(|| {
            packet(
                id,
                u32::from(target.skills.max_hp),
                u32::from(target.skills.hp),
            )
        });
    }
    if !s.visible_entities.contains(&id) {
        return None;
    }
    if let Some(packet) = lobby
        .monsters
        .as_ref()
        .and_then(|w| w.health(id, s.position.zone))
    {
        return Some(packet);
    }
    let npc = lobby
        .merchants
        .iter()
        .find(|n| n.id == id && crate::population::same_region_view(n.position, s.position))?;
    let maximum: u32 = lobby
        .db
        .lock()
        .unwrap()
        .query_row(
            "SELECT hp FROM world_spawns WHERE id=?1 AND zone=?2 AND monster=0",
            rusqlite::params![npc.id, s.position.zone],
            |r| r.get(0),
        )
        .ok()?;
    Some(packet(id, maximum.max(1), maximum.max(1)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn npc_bars_and_damage_snapshots_use_native_widths_and_immediate_updates() {
        let Some((mut lobby, mut s)) = crate::world_service_tests::setup() else {
            return;
        };
        {
            let db = lobby.db.lock().unwrap();
            crate::population::tests::seed(&db);
            db.execute(
                "UPDATE world_spawns SET monster=0,kind=31,hp=7500 WHERE id=12000",
                [],
            )
            .unwrap();
            lobby.merchants = crate::shops::load_population(&db, &reko_common::game_dir()).unwrap();
        }
        s.visible_entities.insert(12000);
        let npc = query(&lobby, &s, 12000).unwrap();
        assert_eq!(npc, packet(12000, 7500, 7500));
        s.visible_entities.clear();
        assert!(query(&lobby, &s, 12000).is_none());
        let world =
            std::sync::Arc::new(crate::monsters::World::load(&reko_common::game_dir()).unwrap());
        let mut original = world.health(10000, s.position.zone).unwrap();
        original[3] = 0;
        s.visible_entities.insert(10000);
        lobby.monsters = Some(world);
        assert!(query(&lobby, &s, 10000).is_some());
        let event = crate::monsters::Event {
            personal_hp: Vec::new(),
            zone: s.position.zone,
            packet: original.clone(),
        };
        let now = crate::population::event_packet(&event, &lobby, &mut s).unwrap();
        assert_eq!(now[3], 1);
        original[3] = 1;
        assert_eq!(now, original, "no HP or damage fields may change");
        let damage = crate::monsters::Event {
            personal_hp: vec![(s.character.name.clone(), -123), ("Friend".into(), -456)],
            zone: s.position.zone,
            packet: original.clone(),
        };
        let own = crate::population::event_packet(&damage, &lobby, &mut s).unwrap();
        assert_eq!(&own[4..12], &original[4..12]);
        assert_eq!(i16::from_le_bytes(own[12..14].try_into().unwrap()), -123);
        s.character.name = "Friend".into();
        let friend = crate::population::event_packet(&damage, &lobby, &mut s).unwrap();
        assert_eq!(&friend[4..12], &own[4..12]);
        assert_eq!(i16::from_le_bytes(friend[12..14].try_into().unwrap()), -456);
        s.character.name = "Bystander".into();
        let observer = crate::population::event_packet(&damage, &lobby, &mut s).unwrap();
        assert_eq!(&observer[4..12], &own[4..12]);
        assert_eq!(&observer[12..14], &[0, 0]);
        s.position.zone = 1;
        assert!(query(&lobby, &s, 12000).is_none());
    }
}
