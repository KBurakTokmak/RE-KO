//! Player-vs-Player combat, damage scaling, death/NP rewards and curses.
//!
//! Damage formulas and zone hostility follow supplied 1453 `GameServer/Unit.cpp`
//! (`GetDamage`, `isHostileTo`, `isInSafetyArea`), `AttackHandler.cpp`,
//! `MagicInstance.cpp` (`GetMagicDamage`, PvP /2 physical and /3 magic scaling),
//! and `User.cpp` (`HpChange`, `OnDeath`, `LoyaltyChange`).

use crate::{
    challenge, combat, lobby::Lobby, monsters, movement::Position, quest_rewards, skills,
    social::Hub, target_hp, utility, world_entry::Session,
};
use rusqlite::params;

pub(crate) const fn is_player_id(id: u16) -> bool {
    id < 10_000
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Hit {
    pub damage: u32,
    pub dead: bool,
}

fn next_rand(seed: &mut u64, max: u32) -> u32 {
    if max == 0 {
        return 0;
    }
    *seed = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    ((*seed >> 16) as u32) % (max + 1)
}

pub(crate) fn in_safety_area(pos: Position, nation: u8) -> bool {
    let (x, z) = (pos.x, pos.z);
    match pos.zone {
        challenge::ZONE_ARENA => challenge::in_arena_safety_area(pos),
        31 => {
            if nation == 2 {
                x > 560 && x < 1240 && z > 7000 && z < 8400
            } else {
                x > 1900 && x < 2700 && z > 8700 && z < 9700
            }
        }
        1 => nation == 1 && x > 18200 && x < 18760 && z > 1360 && z < 2120,
        2 => nation == 2 && x > 1760 && x < 2440 && z > 18200 && z < 18800,
        _ => false,
    }
}

pub(crate) fn is_hostile(hub: &Hub, caster: &Session, target: &Session) -> bool {
    if caster.player_id == target.player_id
        || !caster.entered
        || caster.zone_loading
        || caster.skills.hp == 0
        || !target.entered
        || target.zone_loading
        || target.skills.hp == 0
        || caster.position.zone != target.position.zone
        || target.is_admin
    {
        return false;
    }
    if in_safety_area(caster.position, caster.character.nation())
        || in_safety_area(target.position, target.character.nation())
    {
        return false;
    }
    match caster.position.zone {
        challenge::ZONE_ARENA => {
            let same_party = hub
                .party(caster.player_id)
                .is_some_and(|p| hub.party(target.player_id) == Some(p));
            let same_clan = caster.clan.id != 0 && caster.clan.id == target.clan.id;
            !same_party && !same_clan
        }
        31 | 61..=69 | 71 | 72 => caster.character.nation() != target.character.nation(),
        _ => false,
    }
}

fn distance_meters(a: Position, b: Position) -> f32 {
    f32::from(a.x.abs_diff(b.x)).hypot(f32::from(a.z.abs_diff(b.z))) / 10.0
}

fn pvp_distance_meters(
    caster: &Session,
    origin: Position,
    target: &Session,
    use_segment: bool,
    now: std::time::Instant,
) -> f32 {
    let caster_curr = caster.movement.current_position(origin, now);
    let target_curr = target.movement.current_position(target.position, now);
    let mut best = distance_meters(origin, target.position)
        .min(distance_meters(caster_curr, target_curr))
        .min(distance_meters(origin, target_curr))
        .min(distance_meters(caster_curr, target.position));
    if use_segment {
        let caster_seg = caster.movement.combat_position(origin, target_curr, now);
        let target_seg = target
            .movement
            .combat_position(target.position, caster_curr, now);
        best = best.min(distance_meters(caster_seg, target_seg));
    }
    best
}

pub(crate) fn can_target(lobby: &Lobby, caster: &Session, target_id: u16) -> bool {
    if !is_player_id(target_id) {
        return false;
    }
    let hub = lobby.social.lock().unwrap();
    hub.can_see(caster.player_id, target_id)
        && hub
            .players
            .get(&target_id)
            .is_some_and(|t| is_hostile(&hub, caster, &t.s))
}

pub(crate) fn validate_target(
    lobby: &Lobby,
    caster: &Session,
    target_id: u16,
    origin: Position,
    range: f32,
) -> Result<(), &'static str> {
    if !is_player_id(target_id) {
        return Err("Unknown combat target.");
    }
    let hub = lobby.social.lock().unwrap();
    let target = hub
        .players
        .get(&target_id)
        .ok_or("Unknown combat target.")?;
    if !hub.can_see(caster.player_id, target_id) || !is_hostile(&hub, caster, &target.s) {
        return Err("You cannot attack this player here.");
    }
    if pvp_distance_meters(caster, origin, &target.s, false, std::time::Instant::now())
        > range + 4.5
    {
        return Err("The target is too far away.");
    }
    Ok(())
}

pub(crate) fn area_targets(
    lobby: &Lobby,
    caster: &Session,
    center: Position,
    radius: f32,
) -> Vec<u16> {
    let now = std::time::Instant::now();
    let hub = lobby.social.lock().unwrap();
    hub.players
        .iter()
        .filter_map(|(&id, player)| {
            let target_curr = player.s.movement.current_position(player.s.position, now);
            let dist = distance_meters(center, player.s.position)
                .min(distance_meters(center, target_curr));
            (hub.can_see(caster.player_id, id)
                && is_hostile(&hub, caster, &player.s)
                && dist <= radius + 4.5)
                .then_some(id)
        })
        .collect()
}

fn hit_rate_tier(ratio: f32, seed: &mut u64) -> bool {
    let success = if ratio >= 5.0 {
        9800
    } else if ratio >= 3.0 {
        9600
    } else if ratio >= 2.0 {
        9400
    } else if ratio >= 1.25 {
        9200
    } else if ratio >= 0.8 {
        9000
    } else if ratio >= 0.5 {
        8000
    } else if ratio >= 0.33 {
        7000
    } else if ratio >= 0.2 {
        6000
    } else {
        5000
    };
    next_rand(seed, 9999) < success
}

fn compute_damage(
    caster: &Session,
    target: &Session,
    target_hp: u16,
    attack: &monsters::Attack,
    seed: &mut u64,
) -> u32 {
    if attack.drain_percent > 0 {
        return (u32::from(target_hp) * u32::from(attack.drain_percent) / 100).max(1);
    }
    let now = skills::now_ms();
    let mut total = 0u32;

    if attack.attack > 0 {
        let ac = u32::from(target.combat.defence);
        // 1453 Unit.cpp:254: Rogue Critical Point (category 19) doubles physical AP in PvP (players only).
        let cp_mult = if caster.skills.buffs.get(&19).is_some_and(|b| now < b.until) {
            2u32
        } else {
            1u32
        };
        let eff_attack = u32::from(attack.attack).saturating_mul(cp_mult);
        // 1453 Unit.cpp:244,268: temp_ap = attack * 100; temp_hit_B = (temp_ap * 200 / 100) / (ac + 240)
        let temp_hit_b = eff_attack * 200 / (ac + 240);
        let acc_buff = caster.skills.buffs.get(&9).filter(|b| now < b.until);
        let concentration = acc_buff.is_some_and(|b| matches!(b.skill, 108775 | 208775));
        let illusion = acc_buff.is_some_and(|b| matches!(b.skill, 108630 | 208630));
        let hit_ok = if concentration {
            true
        } else if illusion {
            let ratio = caster.combat.hit_rate / target.combat.evasion.max(0.001) * 0.30;
            hit_rate_tier(ratio, seed)
        } else if attack.skill {
            true
        } else {
            let ratio = caster.combat.hit_rate / target.combat.evasion.max(0.001);
            hit_rate_tier(ratio, seed)
        };
        if hit_ok {
            let phys = if attack.skill {
                let temp_hit = if attack.arrow && attack.penetrate {
                    eff_attack * u32::from(attack.percent) / 100
                } else {
                    temp_hit_b * u32::from(attack.percent) / 100
                };
                let rand = next_rand(seed, temp_hit);
                if attack.arrow {
                    ((temp_hit as f32 * 0.6) + rand as f32 + 0.99) as u32
                } else {
                    ((temp_hit as f32) + 0.3 * rand as f32 + 0.99) as u32
                }
            } else {
                let rand = next_rand(seed, temp_hit_b);
                (0.85 * temp_hit_b as f32 + 0.30 * rand as f32) as u32
            };
            let extra = u32::from(attack.extra)
                / if matches!(caster.position.zone, 61..=69) {
                    2
                } else {
                    3
                };
            // Unit::GetDamage halves physical damage before ExecuteType1 adds its flat bonus.
            total = total
                .saturating_add((phys / 2).max(1))
                .saturating_add(extra);
        }
    }

    if let Some(magic) = attack.magic
        && magic.first > 0
    {
        let mut elem_dmg = if magic.raw_first {
            u32::from(magic.first)
        } else {
            let resist = if magic.element >= 1 && magic.element <= 6 {
                u32::from(target.combat.resistance[usize::from(magic.element - 1)])
            } else {
                0
            };
            let scaled = u32::from(magic.first) * u32::from(magic.charisma) / 186;
            let base = 230 * scaled / (resist + 250);
            let rand = next_rand(seed, base);
            let mut raw = (0.85 * base as f32 + 0.30 * rand as f32) as u32;
            raw = raw.saturating_add(u32::from(magic.charisma.saturating_sub(86)));
            raw = raw.saturating_add(u32::from(magic.bonus));
            // 1453 MagicInstance.cpp:2743-2744: PvP magic damage is divided by 3.
            (raw / 3).max(1)
        };
        // 1453 MagicProcess.cpp:456: Priest Elysian Web (category 27) reduces incoming magic damage by 30%.
        if target.skills.buffs.get(&27).is_some_and(|b| now < b.until) {
            elem_dmg = (elem_dmg * 70 / 100).max(1);
        }
        total = total.saturating_add(elem_dmg);
    }

    total.min(32767)
}

fn periodic_tick_amount(target: &Session, magic: monsters::MagicDamage) -> Option<(i16, u64)> {
    if magic.over_time == 0 || magic.duration == 0 {
        return None;
    }
    let duration_ms = u64::from(magic.duration) * 1000;
    let ticks = (u32::from(magic.duration) / 2).max(1);
    let total = if magic.raw_over_time {
        u32::from(magic.over_time)
    } else {
        let resist = if magic.element >= 1 && magic.element <= 6 {
            u32::from(target.combat.resistance[usize::from(magic.element - 1)])
        } else {
            0
        };
        let scaled = u32::from(magic.over_time) * u32::from(magic.charisma) / 186;
        (230 * scaled / (resist + 250)) / 3
    };
    let per_tick = (total / ticks).clamp(1, 32767) as i16;
    Some((-per_tick, duration_ms))
}

fn loyalty_packet(loyalty: u32, monthly: u32) -> Vec<u8> {
    let mut p = vec![0x2a, 1];
    p.extend(loyalty.to_le_bytes());
    p.extend(monthly.to_le_bytes());
    p
}

pub(crate) fn user_look_change(player_id: u16, slot: u8, item_id: u32, durability: u16) -> Vec<u8> {
    let mut p = vec![0x2d];
    p.extend(player_id.to_le_bytes());
    p.push(slot);
    p.extend(item_id.to_le_bytes());
    p.extend(durability.to_le_bytes());
    p
}

fn is_shield(catalog: Option<&combat::Catalog>, item_id: u32) -> bool {
    catalog
        .and_then(|c| c.item(item_id))
        .is_some_and(|(base, _)| combat::integer(base, 10) / 10 == 6)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn hit_player(
    lobby: &Lobby,
    attacker_account: &str,
    caster: &Session,
    target_id: u16,
    attack: monsters::Attack,
    use_movement_segment: bool,
    debuff: Option<(u8, u32, u64)>,
    charge: impl FnOnce() -> Result<(), &'static str>,
) -> Result<Hit, &'static str> {
    if !is_player_id(target_id) {
        return Err("Unknown combat target.");
    }
    let now = skills::now_ms();
    // Phase 1: pre-validate hostility and range before spending caster resources.
    {
        let hub = lobby.social.lock().unwrap();
        let target = hub
            .players
            .get(&target_id)
            .ok_or("Unknown combat target.")?;
        if !hub.can_see(caster.player_id, target_id) || !is_hostile(&hub, caster, &target.s) {
            return Err("You cannot attack this player here.");
        }
        let max_range = if attack.skill {
            attack.range + 4.5
        } else {
            // Basic R melee reach against another player with movement-sync headroom.
            ((attack.range + 1.0) * 1.25).max(4.5) + 1.5
        };
        if pvp_distance_meters(
            caster,
            attack.position,
            &target.s,
            use_movement_segment,
            std::time::Instant::now(),
        ) > max_range
        {
            return Err("The target is too far away.");
        }
    }

    // Phase 2: commit caster resources (mana/arrows/cooldowns) before locking db+social together.
    charge()?;

    // Phase 3: lock `db` before `social` (deadlock-free order) and apply damage atomically.
    let mut db = lobby.db.lock().unwrap();
    let mut hub = lobby.social.lock().unwrap();
    let Some(target_player) = hub.players.get(&target_id).cloned() else {
        return Err("The target is no longer online.");
    };
    if !hub.can_see(caster.player_id, target_id) || !is_hostile(&hub, caster, &target_player.s) {
        return Err("You cannot attack this player here.");
    }

    let mut seed = now
        .wrapping_mul(6364136223846793005)
        .wrapping_add(u64::from(caster.player_id))
        .wrapping_add(u64::from(target_id) << 16);

    struct ReflectOutcome {
        caster_next: skills::State,
        damage: u32,
        dead: bool,
    }

    struct HitOutcome {
        next: skills::State,
        actual: u32,
        dead: bool,
        mana_changed: bool,
        stealth_broken: bool,
        periodic_key: Option<u32>,
        debuff_applied: Option<(u8, u32, u64)>,
        reflected: Option<ReflectOutcome>,
        killer_np: Option<(u32, u32)>,
        victim_np: Option<(u32, u32)>,
    }

    let result = (|| -> anyhow::Result<HitOutcome> {
        let tx = db.transaction()?;
        let mut next = skills::load(&tx, &target_player.account, target_player.s.character.slot)?;
        if next.hp == 0 {
            anyhow::bail!("The target is already dead.");
        }
        let raw_damage = compute_damage(caster, &target_player.s, next.hp, &attack, &mut seed);
        let before_mana = next.mana;
        let absorbed = utility::damage_received(
            utility::absorb_damage_public(
                &mut next,
                raw_damage.min(65535) as u16,
                lobby.skills.as_ref(),
            ),
            &target_player.s,
        );
        let actual = u32::from(absorbed).min(u32::from(next.hp));
        next.hp = next.hp.saturating_sub(absorbed);
        let dead = next.hp == 0;
        let mana_changed = next.mana != before_mana;
        let stealth_broken = actual > 0 && next.buffs.remove(&91).is_some();

        let reflected = if actual > 0 && next.buffs.get(&25).is_some_and(|b| now < b.until) {
            next.buffs.remove(&25);
            let mut caster_next = skills::load(&tx, attacker_account, caster.character.slot)?;
            if caster_next.hp > 0 {
                // Owner requirement: Mage 75 Armor reflects flat unreduced 2000 damage in PvP.
                const MAGE_ARMOR_FLAT_DAMAGE: u16 = 2000;
                let caster_actual =
                    u32::from(MAGE_ARMOR_FLAT_DAMAGE).min(u32::from(caster_next.hp));
                caster_next.hp = caster_next.hp.saturating_sub(MAGE_ARMOR_FLAT_DAMAGE);
                let caster_dead = caster_next.hp == 0;
                if caster_dead {
                    crate::travel::clear_zone_effects(&mut caster_next, lobby.skills.as_ref());
                }
                skills::save_in(&tx, attacker_account, caster.character.slot, &caster_next)?;
                Some(ReflectOutcome {
                    caster_next,
                    damage: caster_actual,
                    dead: caster_dead,
                })
            } else {
                None
            }
        } else {
            None
        };

        let mut debuff_applied = None;
        let mut periodic_key = None;
        if dead {
            crate::travel::clear_zone_effects(&mut next, lobby.skills.as_ref());
        } else {
            if let Some(magic) = attack.magic
                && (attack.attack == 0 || actual > 0)
                && let Some((tick_amt, duration_ms)) = periodic_tick_amount(&target_player.s, magic)
            {
                periodic_key = next.track_periodic_damage(
                    magic.element,
                    skills::Periodic {
                        amount: tick_amt,
                        next: now + 2000,
                        until: now + duration_ms,
                    },
                );
            }
            if let Some((cat, skill_id, duration_ms)) = debuff
                && actual > 0
                && !next.buffs.get(&29).is_some_and(|b| now < b.until)
                && !next.buffs.get(&30).is_some_and(|b| now < b.until)
                && !(matches!(skill_id, 105520 | 106520 | 205520 | 206520)
                    && next.slowed(lobby.skills.as_ref(), now))
            {
                let resisted = if cat == 6 && attack.magic.is_some_and(|m| m.element == 2) {
                    let cold_r = u32::from(target_player.s.combat.resistance[1]);
                    let chance = 100u32.saturating_sub(cold_r * 100 / 250).clamp(15, 75);
                    next_rand(&mut seed, 99) >= chance
                } else if cat == 47
                    && !matches!(skill_id, 106820 | 206820)
                    && attack.magic.is_some_and(|m| m.element == 3)
                {
                    let light_r = u32::from(target_player.s.combat.resistance[2]);
                    let chance = 100u32.saturating_sub(light_r * 100 / 250).clamp(10, 45);
                    next_rand(&mut seed, 99) >= chance
                } else {
                    false
                };
                if !resisted {
                    crate::travel::suspend_scroll_for_debuff(
                        &mut next,
                        cat,
                        now,
                        lobby.skills.as_ref(),
                    );
                    next.buffs.insert(
                        cat,
                        skills::Buff {
                            skill: skill_id,
                            until: now + duration_ms,
                        },
                    );
                    debuff_applied = Some((cat, skill_id, duration_ms));
                }
            }
        }

        skills::save_in(
            &tx,
            &target_player.account,
            target_player.s.character.slot,
            &next,
        )?;

        let mut killer_np = None;
        let mut victim_np = None;
        if dead
            && matches!(caster.position.zone, 71 | 72)
            && caster.character.nation() != target_player.s.character.nation()
        {
            let mut v_tot =
                quest_rewards::load(&tx, &target_player.account, target_player.s.character.slot)?;
            if v_tot.loyalty > 0 {
                let mut k_tot = quest_rewards::load(&tx, attacker_account, caster.character.slot)?;
                v_tot.loyalty = v_tot.loyalty.saturating_sub(25);
                k_tot.loyalty = k_tot.loyalty.saturating_add(50).min(2_100_000_000);
                k_tot.monthly = k_tot.monthly.saturating_add(50).min(2_100_000_000);
                tx.execute(
                    "INSERT INTO character_quest_rewards VALUES(?1,?2,?3,?4,?5) ON CONFLICT(account,slot) DO UPDATE SET experience=excluded.experience,loyalty=excluded.loyalty,monthly=excluded.monthly",
                    params![
                        &target_player.account,
                        target_player.s.character.slot,
                        v_tot.experience,
                        v_tot.loyalty,
                        v_tot.monthly
                    ],
                )?;
                tx.execute(
                    "INSERT INTO character_quest_rewards VALUES(?1,?2,?3,?4,?5) ON CONFLICT(account,slot) DO UPDATE SET experience=excluded.experience,loyalty=excluded.loyalty,monthly=excluded.monthly",
                    params![
                        attacker_account,
                        caster.character.slot,
                        k_tot.experience,
                        k_tot.loyalty,
                        k_tot.monthly
                    ],
                )?;
                killer_np = Some((k_tot.loyalty, k_tot.monthly));
                victim_np = Some((v_tot.loyalty, v_tot.monthly));
            }
        }

        tx.commit()?;
        Ok(HitOutcome {
            next,
            actual,
            dead,
            mana_changed,
            stealth_broken,
            periodic_key,
            debuff_applied,
            reflected,
            killer_np,
            victim_np,
        })
    })();

    let HitOutcome {
        next,
        actual,
        dead,
        mana_changed,
        stealth_broken,
        periodic_key,
        debuff_applied,
        reflected,
        killer_np,
        victim_np,
    } = match result {
        Ok(ok) => ok,
        Err(_) => return Err("The target is unavailable."),
    };

    hub.skill_refresh.insert(target_id);
    if let Some(p) = hub.players.get_mut(&target_id) {
        p.s.skills = next.clone();
    }

    // Victim HP (and MP if Mana Shield absorbed damage)
    hub.send(target_id, next.hp_packet_from(caster.player_id));
    if mana_changed {
        hub.send(target_id, next.mana_packet());
    }
    if let Some(key) = periodic_key {
        hub.periodic_sources
            .insert((target_id, key), caster.player_id);
        hub.send(target_id, vec![0x5e, 2, skills::damage_status(key), 1]);
    }
    if let Some((category, skill_id, duration_ms)) = debuff_applied {
        let mut effect = skills::effect(skill_id, caster.player_id);
        effect[8..10].copy_from_slice(&target_id.to_le_bytes());
        effect[12..14].copy_from_slice(&1u16.to_le_bytes());
        let duration_s = (duration_ms / 1000).min(32767) as u16;
        effect[16..18].copy_from_slice(&duration_s.to_le_bytes());
        if let Some(row) = lobby
            .skills
            .as_ref()
            .and_then(|c| c.effects.get(&(4, skill_id)))
        {
            let speed = combat::integer(row, 5).clamp(0, 200) as u16;
            effect[20..22].copy_from_slice(&speed.to_le_bytes());
        }
        hub.send(target_id, effect);
        if category == 20 {
            hub.send(target_id, vec![0x5e, 2, 4, 1]);
        }
        if matches!(category, 6 | 40 | 47) {
            hub.send(target_id, vec![0x5e, 2, 3, 1]);
        }
        if category == 47 {
            let mut stop_pkt = vec![0x06];
            stop_pkt.extend(target_id.to_le_bytes());
            stop_pkt.extend(target_player.s.position.x.to_le_bytes());
            stop_pkt.extend(target_player.s.position.z.to_le_bytes());
            stop_pkt.extend(target_player.s.position.y.to_le_bytes());
            stop_pkt.extend(0u16.to_le_bytes());
            stop_pkt.push(0);
            hub.broadcast_visual(target_id, None, None, &stop_pkt);
        }
    }
    if stealth_broken {
        hub.send(target_id, vec![0x31, 5, 91]);
        crate::magic::visibility(lobby, &target_player.s, false);
    }
    if let Some(refl) = reflected {
        hub.send(target_id, vec![0x31, 5, 25]);
        hub.skill_refresh.insert(caster.player_id);
        if let Some(p) = hub.players.get_mut(&caster.player_id) {
            p.s.skills = refl.caster_next.clone();
        }
        hub.send(caster.player_id, refl.caster_next.hp_packet_from(target_id));
        let refl_delta = -(refl.damage.min(32767) as i16);
        let mut target_refl_hp = target_hp::packet(
            caster.player_id,
            u32::from(refl.caster_next.max_hp),
            u32::from(refl.caster_next.hp),
        );
        target_refl_hp[12..14].copy_from_slice(&refl_delta.to_le_bytes());
        hub.send(target_id, target_refl_hp);
        hub.notify_target_hp(
            caster.player_id,
            target_id,
            u32::from(refl.caster_next.max_hp),
            u32::from(refl.caster_next.hp),
        );
        if refl.dead {
            hub.player_died(caster.player_id);
        }
    }

    // TargetHP bar & personal damage log for attacker; delta 0 for bystanders
    let delta = -(actual.min(32767) as i16);
    let mut attacker_hp_pkt =
        target_hp::packet(target_id, u32::from(next.max_hp), u32::from(next.hp));
    attacker_hp_pkt[12..14].copy_from_slice(&delta.to_le_bytes());
    hub.send(caster.player_id, attacker_hp_pkt);
    hub.notify_target_hp(
        target_id,
        caster.player_id,
        u32::from(next.max_hp),
        u32::from(next.hp),
    );

    if dead {
        hub.player_died(target_id);
    }

    if let Some((loyalty, monthly)) = killer_np {
        if let Some(p) = hub.players.get_mut(&caster.player_id) {
            p.s.rewards.loyalty = loyalty;
            p.s.rewards.monthly = monthly;
        }
        hub.send(caster.player_id, loyalty_packet(loyalty, monthly));
    }
    if let Some((loyalty, monthly)) = victim_np {
        if let Some(p) = hub.players.get_mut(&target_id) {
            p.s.rewards.loyalty = loyalty;
            p.s.rewards.monthly = monthly;
        }
        hub.send(target_id, loyalty_packet(loyalty, monthly));
    }

    Ok(Hit {
        damage: actual,
        dead,
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn curse_player(
    lobby: &Lobby,
    caster: &Session,
    target_id: u16,
    origin: Position,
    range: f32,
    category: u8,
    skill_id: u32,
    duration_ms: u64,
    charge: impl FnOnce() -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    if !is_player_id(target_id) {
        return Err("Unknown combat target.");
    }
    {
        let hub = lobby.social.lock().unwrap();
        let target = hub
            .players
            .get(&target_id)
            .ok_or("Unknown combat target.")?;
        if !hub.can_see(caster.player_id, target_id) || !is_hostile(&hub, caster, &target.s) {
            return Err("You cannot curse this player here.");
        }
        if pvp_distance_meters(caster, origin, &target.s, false, std::time::Instant::now())
            > range + 4.5
        {
            return Err("The target is too far away.");
        }
    }
    charge()?;

    let now = skills::now_ms();
    let mut db = lobby.db.lock().unwrap();
    let mut hub = lobby.social.lock().unwrap();
    let Some(target_player) = hub.players.get(&target_id).cloned() else {
        return Err("The target is no longer online.");
    };
    if !hub.can_see(caster.player_id, target_id) || !is_hostile(&hub, caster, &target_player.s) {
        return Err("You cannot curse this player here.");
    }
    let result = (|| -> anyhow::Result<Option<(skills::State, combat::Stats, bool)>> {
        let tx = db.transaction()?;
        let mut next = skills::load(&tx, &target_player.account, target_player.s.character.slot)?;
        if next.hp == 0 {
            anyhow::bail!("The target is dead.");
        }
        if next.buffs.get(&29).is_some_and(|b| now < b.until)
            || next.buffs.get(&30).is_some_and(|b| now < b.until)
        {
            return Ok(None);
        }
        crate::travel::suspend_scroll_for_debuff(&mut next, category, now, lobby.skills.as_ref());
        next.buffs.insert(
            category,
            skills::Buff {
                skill: skill_id,
                until: now + duration_ms,
            },
        );
        let stealth_broken = category == 26 && next.buffs.remove(&91).is_some();
        let stats = utility::effective(
            &target_player.s,
            &next,
            lobby.skills.as_ref(),
            lobby.combat.as_ref(),
        );
        if matches!(category, 1 | 7) {
            next.set_limits(stats.hp, stats.mp);
        }
        skills::save_in(
            &tx,
            &target_player.account,
            target_player.s.character.slot,
            &next,
        )?;
        tx.commit()?;
        Ok(Some((next, stats, stealth_broken)))
    })();

    let Ok(applied) = result else {
        return Err("Unable to apply curse.");
    };
    if let Some((next, stats, stealth_broken)) = applied {
        hub.skill_refresh.insert(target_id);
        if let Some(p) = hub.players.get_mut(&target_id) {
            p.s.skills = next.clone();
            p.s.combat = stats;
        }
        let mut effect = skills::effect(skill_id, caster.player_id);
        effect[8..10].copy_from_slice(&target_id.to_le_bytes());
        effect[12..14].copy_from_slice(&1u16.to_le_bytes());
        let duration_s = (duration_ms / 1000).min(32767) as u16;
        effect[16..18].copy_from_slice(&duration_s.to_le_bytes());
        hub.send(target_id, effect);
        hub.send(target_id, next.hp_packet(0));
        hub.send(target_id, crate::inventory::attributes(&stats, 2));
        if stealth_broken {
            hub.send(target_id, vec![0x31, 5, 91]);
            crate::magic::visibility(lobby, &target_player.s, false);
        }
        if category == 32 {
            let rh = user_look_change(target_id, 6, 0, 0);
            hub.broadcast_visual(target_id, None, Some(target_id), &rh);
            hub.send(target_id, rh);
            let lh_item = target_player.s.equipment[8];
            if lh_item.id != 0 && !is_shield(lobby.combat.as_ref(), lh_item.id) {
                let lh = user_look_change(target_id, 8, 0, 0);
                hub.broadcast_visual(target_id, None, Some(target_id), &lh);
                hub.send(target_id, lh);
            }
        }
    }
    Ok(())
}

pub(crate) fn drain_player_mana(
    lobby: &Lobby,
    caster: &Session,
    target_id: u16,
    origin: Position,
    range: f32,
    amount: u16,
    charge: impl FnOnce() -> Result<(), &'static str>,
) -> Result<u32, &'static str> {
    if !is_player_id(target_id) {
        return Err("Unknown combat target.");
    }
    {
        let hub = lobby.social.lock().unwrap();
        let target = hub
            .players
            .get(&target_id)
            .ok_or("Unknown combat target.")?;
        if !hub.can_see(caster.player_id, target_id) || !is_hostile(&hub, caster, &target.s) {
            return Err("You cannot attack this player here.");
        }
        if pvp_distance_meters(caster, origin, &target.s, false, std::time::Instant::now())
            > range + 4.5
        {
            return Err("The target is too far away.");
        }
    }
    charge()?;
    let mut db = lobby.db.lock().unwrap();
    let mut hub = lobby.social.lock().unwrap();
    let Some(target_player) = hub.players.get(&target_id).cloned() else {
        return Err("The target is no longer online.");
    };
    let result = (|| -> anyhow::Result<(skills::State, u32)> {
        let tx = db.transaction()?;
        let mut next = skills::load(&tx, &target_player.account, target_player.s.character.slot)?;
        if next.hp == 0 {
            anyhow::bail!("The target is dead.");
        }
        let before = next.mana;
        next.mana = next.mana.saturating_sub(amount);
        let drained = u32::from(before - next.mana);
        skills::save_in(
            &tx,
            &target_player.account,
            target_player.s.character.slot,
            &next,
        )?;
        tx.commit()?;
        Ok((next, drained))
    })();
    let Ok((next, drained)) = result else {
        return Err("Unable to drain target mana.");
    };
    hub.skill_refresh.insert(target_id);
    if let Some(p) = hub.players.get_mut(&target_id) {
        p.s.skills = next.clone();
    }
    hub.send(target_id, next.mana_packet());
    Ok(drained)
}

/// Exceed Break uses the supplied 1453 50% roll and the original tooltip's
/// armor-only 1,000 durability amount.
pub(crate) fn break_armor(lobby: &Lobby, target_id: u16, amount: u16) {
    break_armor_on_roll(lobby, target_id, amount, skills::now_ms());
}

fn break_armor_on_roll(lobby: &Lobby, target_id: u16, amount: u16, roll: u64) {
    if !is_player_id(target_id) || roll.is_multiple_of(2) {
        return;
    }
    let mut db = lobby.db.lock().unwrap();
    let mut hub = lobby.social.lock().unwrap();
    let Some(target_player) = hub.players.get(&target_id).cloned() else {
        return;
    };
    let Ok(tx) = db.transaction() else {
        return;
    };
    let Ok(mut gear) =
        crate::starter::equipment(&tx, &target_player.account, target_player.s.character.slot)
    else {
        return;
    };
    // Armor slots: helmet(1), pauldron(4), pads(10), gauntlets(12), boots(13)
    const ARMOR_SLOTS: [usize; 5] = [1, 4, 10, 12, 13];
    let equipped: Vec<usize> = ARMOR_SLOTS
        .into_iter()
        .filter(|&s| gear[s].id != 0 && gear[s].durability > 0)
        .collect();
    if equipped.is_empty() {
        return;
    }
    let slot = equipped[(roll as usize / 2) % equipped.len()];
    let next_dur = gear[slot].durability.saturating_sub(amount);
    if tx
            .execute(
                "UPDATE character_equipment SET durability=?1 WHERE account=?2 AND slot=?3 AND equipment_slot=?4 AND item_id=?5",
                params![next_dur, &target_player.account, target_player.s.character.slot, slot as u8, gear[slot].id],
            )
            .is_ok_and(|rows| rows == 1)
        && tx.commit().is_ok()
    {
        gear[slot].durability = next_dur;
        let attr_pkt = hub.players.get_mut(&target_id).map(|p| {
            p.s.equipment = gear;
            if let Some(items) = lobby.combat.as_ref()
                && let Ok(mut base) =
                    items.calculate(&p.s.character, &p.s.progress, &p.s.equipment)
            {
                base.weight = base.weight.saturating_add(items.bag_weight(&p.s.inventory));
                p.s.base_combat = base;
            }
            p.s.combat = utility::effective(
                &p.s,
                &p.s.skills,
                lobby.skills.as_ref(),
                lobby.combat.as_ref(),
            );
            crate::inventory::attributes(&p.s.combat, 2)
        });
        if let Some(pkt) = attr_pkt {
            hub.send(target_id, pkt);
        }
        hub.refresh.insert(target_id);
        let mut dur_pkt = vec![0x38, slot as u8];
        dur_pkt.extend(next_dur.to_le_bytes());
        hub.send(target_id, dur_pkt);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        characters::{Character, CreationRules},
        social,
        world_entry::Bootstrap,
    };
    use rusqlite::Connection;
    use std::sync::Arc;

    fn test_lobby() -> Arc<Lobby> {
        Arc::new(
            Lobby::new(
                Connection::open_in_memory().unwrap(),
                CreationRules::fixture(),
                Bootstrap { next_level_exp: 50 },
            )
            .unwrap(),
        )
    }

    fn spawn(
        lobby: &Lobby,
        account: &str,
        name: &str,
        nation: u8,
        zone: u8,
        x: u16,
        z: u16,
    ) -> Session {
        let (race, class) = if nation == 1 { (1, 106) } else { (11, 206) };
        let ch = Character {
            slot: 0,
            name: name.into(),
            race,
            class,
            face: 0,
            hair: 0,
            stats: [60; 5],
        };
        {
            let mut db = lobby.db.lock().unwrap();
            crate::characters::ensure_account(&db, account).unwrap();
            crate::characters::set_nation(&db, account, nation).unwrap();
            assert_eq!(crate::characters::create(&mut db, account, &ch).unwrap(), 0);
        }
        let mut s = Session::new(ch).unwrap();
        s.entered = true;
        s.position = Position { zone, x, z, y: 0 };
        s.skills.hp = 500;
        s.skills.max_hp = 500;
        s.combat.attack = 1000;
        s.combat.defence = 200;
        s.combat.hit_rate = 100.0;
        s.combat.evasion = 10.0;
        s.combat.range = 2.0;
        skills::save(&lobby.db.lock().unwrap(), account, 0, &s.skills).unwrap();
        social::synchronize(lobby, account, &mut s);
        s
    }

    #[test]
    fn exceed_break_roll_updates_the_persisted_armor_and_native_durability_packet() {
        let lobby = test_lobby();
        let defender = spawn(&lobby, "armor", "ArmorTarget", 2, 48, 1900, 1900);
        lobby
            .db
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO character_equipment VALUES ('armor',0,4,210001008,3500)",
                [],
            )
            .unwrap();
        break_armor_on_roll(&lobby, defender.player_id, 1000, 0);
        break_armor_on_roll(&lobby, defender.player_id, 1000, 1);
        let gear = crate::starter::equipment(&lobby.db.lock().unwrap(), "armor", 0).unwrap();
        assert_eq!(gear[4].durability, 2500);
        let packets = lobby.social.lock().unwrap().take_queue(defender.player_id);
        assert!(packets.contains(&vec![0x38, 4, 0xc4, 0x09]));
    }

    #[test]
    fn blooding_uses_its_full_duration_total_and_poison_arrows_have_distinct_ticks() {
        let lobby = test_lobby();
        let target = spawn(&lobby, "victim", "Target", 2, 48, 1900, 1900);
        let mut magic = monsters::MagicDamage {
            element: 4,
            first: 0,
            over_time: 1000,
            duration: 20,
            charisma: 60,
            bonus: 0,
            raw_over_time: true,
            raw_first: false,
        };
        assert_eq!(periodic_tick_amount(&target, magic), Some((-100, 20_000)));
        magic.element = 6;
        magic.raw_over_time = false;
        let amounts: Vec<_> = [232, 463, 691]
            .into_iter()
            .map(|amount| {
                magic.over_time = amount;
                periodic_tick_amount(&target, magic).unwrap().0
            })
            .collect();
        assert!(amounts.windows(2).all(|two| two[0] > two[1]));
        let mut state = skills::State::default();
        for amount in amounts {
            let key = state
                .track_periodic_damage(
                    6,
                    skills::Periodic {
                        amount,
                        next: 2000,
                        until: 20_000,
                    },
                )
                .unwrap();
            assert_eq!(skills::damage_status(key), 2);
        }
        assert_eq!(state.tick_periodic(2000).len(), 3);
    }

    #[test]
    fn arena_safety_blocks_pvp_while_arena_pits_and_cz_apply_damage_death_and_np() {
        let lobby = test_lobby();
        let mut k = spawn(&lobby, "k1", "KarusWar", 1, 48, 1300, 1150);
        let e = spawn(&lobby, "e1", "ElmoWar", 2, 48, 1310, 1150);

        let make_atk = |pos| monsters::Attack {
            position: pos,
            range: 2.0,
            attack: 1000,
            percent: 300,
            extra: 0,
            skill: true,
            drain_percent: 0,
            arrow: false,
            penetrate: false,
            magic: None,
        };
        // Inside Arena 48 center safety lobby -> refused!
        assert!(
            hit_player(
                &lobby,
                "k1",
                &k,
                e.player_id,
                make_atk(k.position),
                false,
                None,
                || Ok(())
            )
            .is_err()
        );

        // Move both into Arena 48 fighting pit (outside safety lobby) -> PvP damage lands!
        k.position = Position {
            zone: 48,
            x: 1900,
            z: 1900,
            y: 0,
        };
        social::synchronize(&lobby, "k1", &mut k);
        {
            let mut hub = lobby.social.lock().unwrap();
            hub.players.get_mut(&e.player_id).unwrap().s.position = Position {
                zone: 48,
                x: 1910,
                z: 1900,
                y: 0,
            };
        }
        let hit = hit_player(
            &lobby,
            "k1",
            &k,
            e.player_id,
            make_atk(k.position),
            false,
            None,
            || Ok(()),
        )
        .unwrap();
        assert!(
            hit.damage >= 400,
            "expected realistic PvP skill damage, got {}",
            hit.damage
        );
        let saved_hp = skills::load(&lobby.db.lock().unwrap(), "e1", 0).unwrap().hp;
        assert_eq!(u32::from(saved_hp), 500u32.saturating_sub(hit.damage));

        // Colony Zone (71) kill awards +50 NP to killer and -25 NP to victim,
        // clears ordinary buffs, and preserves item-backed scroll buffs.
        let cz_k = spawn(&lobby, "czk", "CzKiller", 1, 71, 5000, 5000);
        let cz_e = spawn(&lobby, "cze", "CzVictim", 2, 71, 5010, 5000);
        {
            let db = lobby.db.lock().unwrap();
            let mut state = skills::load(&db, "cze", 0).unwrap();
            let now = skills::now_ms();
            state.buffs.insert(
                6,
                skills::Buff {
                    skill: 108725,
                    until: now + 60_000,
                },
            );
            state.buffs.insert(
                1,
                skills::Buff {
                    skill: 500024,
                    until: now + 600_000,
                },
            );
            skills::save(&db, "cze", 0, &state).unwrap();
        }
        let lethal = monsters::Attack {
            position: cz_k.position,
            range: 2.0,
            attack: 5000,
            percent: 600,
            extra: 2000,
            skill: true,
            drain_percent: 0,
            arrow: false,
            penetrate: false,
            magic: None,
        };
        let kill = hit_player(
            &lobby,
            "czk",
            &cz_k,
            cz_e.player_id,
            lethal,
            false,
            None,
            || Ok(()),
        )
        .unwrap();
        assert!(kill.dead);
        let db = lobby.db.lock().unwrap();
        assert_eq!(quest_rewards::load(&db, "czk", 0).unwrap().loyalty, 350);
        assert_eq!(quest_rewards::load(&db, "cze", 0).unwrap().loyalty, 275);
        let victim_after = skills::load(&db, "cze", 0).unwrap();
        assert!(!victim_after.buffs.contains_key(&6));
        assert_eq!(
            victim_after.buffs.get(&1).map(|b| b.skill),
            Some(500024),
            "item-backed scroll buff must survive PvP death"
        );
    }

    #[test]
    fn pvp_hit_breaks_stealth_and_armor_cancel_broadcasts_look_change() {
        let lobby = Lobby::new(
            rusqlite::Connection::open_in_memory().unwrap(),
            CreationRules::fixture(),
            Bootstrap { next_level_exp: 50 },
        )
        .unwrap();
        let a = spawn(&lobby, "atk", "Attacker", 1, 71, 5000, 5000);
        let b = spawn(&lobby, "def", "Defender", 2, 71, 5010, 5000);
        {
            let db = lobby.db.lock().unwrap();
            let mut state = skills::load(&db, "def", 0).unwrap();
            state.buffs.insert(
                91,
                skills::Buff {
                    skill: 108700,
                    until: skills::now_ms() + 60_000,
                },
            );
            skills::save(&db, "def", 0, &state).unwrap();
        }
        // Non-lethal Blinding Strafe hit breaks stealth and sends Type 4 blind effect to defender
        let make_poke = || monsters::Attack {
            position: a.position,
            range: 20.0,
            attack: 200,
            percent: 100,
            extra: 10,
            skill: true,
            drain_percent: 0,
            arrow: true,
            penetrate: false,
            magic: None,
        };
        let hit = hit_player(
            &lobby,
            "atk",
            &a,
            b.player_id,
            make_poke(),
            false,
            Some((21, 108580, 3000)),
            || Ok(()),
        )
        .unwrap();
        assert!(hit.damage > 0 && !hit.dead);
        {
            let db = lobby.db.lock().unwrap();
            let def_state = skills::load(&db, "def", 0).unwrap();
            assert!(!def_state.buffs.contains_key(&91));
            assert_eq!(def_state.buffs.get(&21).map(|b| b.skill), Some(108580));
            let q = lobby.social.lock().unwrap().take_queue(b.player_id);
            assert!(
                q.contains(&vec![0x31, 5, 91]),
                "damaged stealthed victim must receive stealth-removal packet"
            );
            assert!(
                q.iter().any(|p| p.len() >= 18
                    && p[0] == 0x31
                    && p[1] == 3
                    && u32::from_le_bytes([p[2], p[3], p[4], p[5]]) == 108580
                    && u16::from_le_bytes([p[12], p[13]]) == 1),
                "victim must receive Type 4 blind buff effect packet"
            );
        }
        // Mage 75 Armor (category 25) reflects flat unreduced 2000 damage back to attacker
        {
            let db = lobby.db.lock().unwrap();
            let mut atk_state = skills::load(&db, "atk", 0).unwrap();
            atk_state.max_hp = 5000;
            atk_state.hp = 5000;
            skills::save(&db, "atk", 0, &atk_state).unwrap();
            let mut def_state = skills::load(&db, "def", 0).unwrap();
            def_state.buffs.insert(
                25,
                skills::Buff {
                    skill: 110573,
                    until: skills::now_ms() + 60_000,
                },
            );
            skills::save(&db, "def", 0, &def_state).unwrap();
        }
        hit_player(
            &lobby,
            "atk",
            &a,
            b.player_id,
            make_poke(),
            false,
            None,
            || Ok(()),
        )
        .unwrap();
        {
            let db = lobby.db.lock().unwrap();
            assert_eq!(
                skills::load(&db, "atk", 0).unwrap().hp,
                3000,
                "Mage 75 Armor must reflect flat unreduced 2000 damage"
            );
            assert!(!skills::load(&db, "def", 0).unwrap().buffs.contains_key(&25));
        }
        // Armor Cancellation (category 32) broadcasts WIZ_USERLOOK_CHANGE (0x2D) to nearby attacker
        let _ = lobby.social.lock().unwrap().take_queue(a.player_id);
        curse_player(
            &lobby,
            &a,
            b.player_id,
            a.position,
            10.0,
            32,
            108820,
            3000,
            || Ok(()),
        )
        .unwrap();
        let aq = lobby.social.lock().unwrap().take_queue(a.player_id);
        assert!(
            aq.contains(&user_look_change(b.player_id, 6, 0, 0)),
            "attacker must receive WIZ_USERLOOK_CHANGE (0x2D) for disarmed right hand"
        );

        // Speed+ scroll (category 6) is suspended under a slow debuff and reinstated on expiry
        let now = skills::now_ms();
        let mut s = skills::State::default();
        s.buffs.insert(
            6,
            skills::Buff {
                skill: 500024,
                until: now + 600_000,
            },
        );
        crate::travel::suspend_scroll_for_debuff(&mut s, 6, now, None);
        s.buffs.insert(
            6,
            skills::Buff {
                skill: 108535,
                until: now + 5_000,
            },
        );
        assert_eq!(s.buffs.get(&6).map(|b| b.skill), Some(108535));
        assert_eq!(s.buffs.get(&106).map(|b| b.skill), Some(500024));
        let expired = s.expire(now + 6_000);
        assert_eq!(expired, vec![6]);
        let restored = crate::travel::restore_suspended_scroll(&mut s, 6, now + 6_000);
        assert_eq!(restored.map(|b| b.skill), Some(500024));
        assert_eq!(s.buffs.get(&6).map(|b| b.skill), Some(500024));
    }

    #[test]
    fn smoke_screen_omits_blind_status_aura_and_staff_element_boosts_meteor_pvp_damage() {
        let Some((mut lobby, mut mage)) = crate::world_service_tests::setup() else {
            return;
        };
        let game = reko_common::game_dir();
        if !game.join("Data/Skill_Magic_Main_us.tbl").exists() {
            return;
        }
        lobby.skills = Some(skills::Catalog::load(&game).unwrap());
        mage.character.class = 110;
        mage.character.stats[4] = 163;
        mage.progress.level = 80;
        mage.position = Position {
            zone: 48,
            x: 1300,
            z: 1200,
            y: 0,
        };
        mage.equipment[6] = crate::starter::Item {
            id: 189101308,
            durability: 13500,
            count: 1,
        };
        mage.skills.set_limits(5000, 5000);
        skills::save(&lobby.db.lock().unwrap(), "test", 0, &mage.skills).unwrap();
        social::synchronize(&lobby, "test", &mut mage);

        let mut rogue = spawn(&lobby, "rogue_def", "RogueTarget", 2, 48, 1305, 1200);
        rogue.combat.resistance = [40; 6];
        let _ = lobby.social.lock().unwrap().take_queue(rogue.player_id);

        curse_player(
            &lobby,
            &mage,
            rogue.player_id,
            mage.position,
            20.0,
            20,
            108780,
            5000,
            || Ok(()),
        )
        .unwrap();
        let rq = lobby.social.lock().unwrap().take_queue(rogue.player_id);
        assert!(
            !rq.contains(&vec![0x5e, 2, 4, 1]),
            "Smoke Screen (category 20) must not emit USER_STATUS_BLIND (purple aura)"
        );

        let row = lobby
            .skills
            .as_ref()
            .unwrap()
            .effects
            .get(&(3, 110571))
            .unwrap();
        let magic = crate::mage::damage(&mage, &lobby, row);
        let attack = monsters::Attack {
            position: mage.position,
            range: 45.0,
            attack: 0,
            percent: 0,
            extra: 0,
            skill: true,
            drain_percent: 0,
            arrow: false,
            penetrate: false,
            magic: Some(magic),
        };
        let mut seed = 1u64;
        let dmg = compute_damage(&mage, &rogue, 5000, &attack, &mut seed);
        assert!(
            dmg >= 650,
            "lv80 Meteor with Wreath of Erenion(+8) must deal >=650 PvP damage (got {dmg})"
        );
    }
}
