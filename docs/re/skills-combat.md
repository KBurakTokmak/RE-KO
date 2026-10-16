# Skills, combat and recovery

Original1453 main/effect tables determine skill IDs, costs, damage percentages,
range, requirements and timing. Supplementary2086 formulas are used where
documented; older1298 values are not automatically equivalent. Core handlers
are `magic.rs`, `archery.rs`, `mage.rs`, `priest.rs`, `utility.rs` and `skills.rs`.

## Supported subsets

| Class | Working paths | Important remaining gaps |
| --- | --- | --- |
| Rogue | Melee damage, percentage drains, Sprint/Swift/Light Feet, stealth, multi-arrow volleys, poison/elemental arrows, self healing/armor/AP/resistance/cures | Other-player buffs, full detection/PvP effects, Illusion/Concentration and displacement effects |
| Warrior | Physical skills, Blooding/periodic recovery, HP Booster, stat/AC/speed buffs, MP absorption, supported defense/master passives | Party movement/sacrifice, provoke/binding, full stun/AI effects |
| Mage | Direct/periodic elemental damage, ground AoE, staff attacks, resistance buffs, Absolute Power, Instantly Magic | Summoning/teleports, reflection, several player-only drains/control effects |
| Priest | Instant/periodic self healing, HP/AC/stat/resistance buffs, cures, physical/offensive skills, NPC AC/max-HP curses | Other-player/party heals, resurrection and several control/counter-curse effects |

Group effects currently support the caster where implemented; that is not a
party system. Unsupported effects fail without charging resources. See the
[backlog](../design/1453-feature-backlog.md) for individual dependencies.

Costs, cooldowns, reagents and persistent effects commit atomically. Cast and
projectile stages cannot charge twice; bounded paid tickets limit impacts and
reject forged/duplicate completions. Multiple Shot/Arrow Shower use original
three/five-arrow counts. Post-launch movement/animation cancellation does not
discard a paid projectile. Movement forecasts alone must not clear a mage cast;
explicit phase4 cancels it. Five-second projectile expiry is an inferred local
policy, not a recovered1453 flight-time constant. Animation minimums need work.

Sprint's original recast60 means six seconds, separate from its ten-second
150percent speed buff. Swift has600seconds/150percent; Light Feet10seconds/
200percent. Recast while a refreshable buff is active is allowed after its own
cooldown. Categories, expiry and remaining time persist through reconnect.

Mage35/54/57 scrolls are ownership requirements, not consumables. Master2 scrolls
379063000..379066000 remain in the bag; respective class stones
379059000..379062000 are consumed. These follow the owner's1453 correction,
supported by the reference's separate required-item/class-stone paths. Missing
requirements roll back costs. Potions retain their own consumption/cooldown rules.

## Damage, equipment and durability

Class/weapon coefficients come from the local coefficient export. Physical
hits use source-backed attack/armor/hit/random paths and original skill values.
Not every item bonus, resistance, weapon-defense requirement or PvP formula is
complete. Imported NPC stats and timed AC/max-HP modifiers affect their damage
and effective maximum without overwriting prototypes.

Supplementary2086 magic damage at00940640 uses charisma/186,
230/(resistance+250),0.85base+0.30random, the charisma-above86 bonus and staff
contribution0.8*staffDamage+trunc(staffDamage*level/60). Non-mages use reference
magic power60. Do not claim these supplementary formulas prove untouched1453
retail equivalence. Weather and full elemental equipment effects remain gaps.

Successful physical hits wear equipped non-shield weapons by
trunc(sqrt(damage/10)), corroborated by Open-KO User::ItemWoreOut and2086.
Misses, pure spells, drains and periodic ticks do not. Durability persists and
repair recalculates attributes. Broken equipment halves its item contribution
following older Open-KO; exact1453 equivalence remains inferred. Defensive armor
wear, appearance thresholds and full killing-hit wear equivalence remain open.

## Recovery

For level L and maximum MP M, supplementary2086 native00A5A650 gives:

- Standing MP: truncate((L*(1+L/60)+1)*0.2+3).
- Sitting HP: truncate(L*(1+L/30))+3.
- Sitting MP: floor(5*M/(L+29))+3.

The source checks whole seconds strictly greater than five, giving the current
six-second cadence. Dead characters do not regenerate; there is no offline
catch-up. Master mage110/210 below30percent MP gets the reference120percent
recovery multiplier. Exact untouched1453 regen remains inferred.

Positive stat allocation preserves current HP/MP rather than silently filling
them; synchronize current and maximum values together. Level-up and supported
max-HP buff application have their own source-backed refill behavior. HP Booster
applies its initial recovery and two-second ticks while standing too.

Monster recovery follows supplementary2086 native0060F970/0062E826:5percent
maximum HP after strictly more than ten seconds. Dead mobs do not regenerate.
Training targets deliberately respawn quickly for testing; those timers are
not historical monster schedules. `hurt N` is restricted to the reusable
character_privileges admin flag and clamps at1HP.
