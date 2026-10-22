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
| Mage | Elemental/AoE/staff damage, NPC lightning stun, self/friendly buffs, transport, bounded player drains/control and NPC reactive armor | Full player damage, exact retail freeze/slow chances, bind points and terrain collision |
| Priest | Instant/periodic self healing, HP/AC/stat/resistance buffs, cures, physical/offensive skills, NPC AC/max-HP curses | Other-player/party heals, resurrection and several control/counter-curse effects |

Most group buffs/heals currently support the caster where implemented; Mage
Escape also transfers eligible party members. Party membership and rewards exist,
but complete group-target buff/heal delivery remains separate. Unsupported effects
fail without charging resources. See the
[backlog](../design/1453-feature-backlog.md) for individual dependencies.

### Transformation Scroll

Original item381001000 invokes list skill472001 (main types0/1). It is not a
recovery spell and now takes a scoped path before the generic item-skill check.
An owned scroll opens native `31 09 <472001:u32>` without consuming an item.
The server remembers a self-bound list request for60seconds. A selected original
472xxx/type6 form must satisfy level, primary class mask, nation, living/self
target and zone/buff restrictions. A selection without that menu, a stale menu,
forged target, duplicate transformation or unavailable scroll cannot charge.

`Import-1453Transformations.py` imports only the46 supplied1453 MAGIC_TYPE6 rows
for the original scroll family, verifying model, one-hour duration and class mask
against original Skill_Magic_6.tbl. Primary Nation0 allows both nations; the
original client column25 is1 for all these rows and must not be misread as a
Karus-only server restriction. Sheriff472160 uses primary mask0010 (Mage only).
The primary fallback in MagicInstance::CheckType6Prerequisites/CheckSkillPrerequisites
allows a single Transformation Scroll instead of requiring a Transformation Gem;
selection atomically consumes one scroll and persists the timed disguise. Failed
payment/state saves roll back inventory. Original combat/stat modifiers are all
zero; no new combat bonuses are introduced. Totems and other scroll families
remain separate, unsupported paths.

Local persisted category96 stores the selected skill and expiry, not a type4 buff.
The actor and nearby players receive state29/type3 with the skill ID, followed
by type6 success with the remaining duration. GM actors first receive the source
visibility reset (state29/type5/normal). Other-player entry packets include the
same abnormal appearance. Manual cancellation sends `31 07` then normal appearance;
expiry, death and entry into restricted zones clear the visual state. Relog and
allowed cross-zone travel replay the remaining duration without another charge.
The current restricted-zone set is31/48/51..69/71..75 (Bifrost, arena, war and
combat zones); home/Eslant remain usable as the primary source permits. Full
war-time map flag changes remain outside this bounded implementation.

Active positive type4 buffs and stealth must be removed before transforming.
The original MonsterFriendly1 behavior suppresses automatic proximity aggression
while retaining retaliation against a transformed attacker; disguise never grants
damage immunity. Basic R now uses the selected type6 model's skeleton radius
scaled by its original size, replacing the human race radius while transformed.
Target scaling, weapon reach, the owner multiplier/minimum and ordinary skill
range rules remain intact. Expired/cancelled/PvP-refused forms use the human
radius again. Form-dependent weapon/animation timing remains a separate limit.

Sources: original item/skill/type6 tables; supplied1453 MagicInstance::SendTransformationList,
CheckType6Prerequisites, ExecuteType6, Type6Cancel and User::StateChangeServerDirect.
Open-KO GameDef::__TABLE_UPC_SKILL_TYPE_6 corroborates client fields with an explicit
warning about later column naming. Socket/SQLite tests exercise all46 forms in both
nations, single scroll payment, GM visibility, class/buff/zone guards, rollback,
replay, cancellation, relog and expiry. AI regression checks disguise versus retaliation.

### Mage integration and original1453 constraints

The original1453 main table has332 Mage records across classes103/109/110 and
203/209/210:6 base,68 advanced and92 master records per nation. Costs,
requirements, timing, types, range and available type1/3/4 effect parameters match
between nation counterparts. Owner reports concern execution, not visibility.
Existing direct/periodic elemental, ground AoE and staff paths remain intact.
The socket regression now verifies level80 Igzination110575/210575, including
single payment and rejection of repeated projectile completion.

| Skills | Current execution and limits |
| --- | --- |
| Summon Friend, Gate, Escape, Blink | `mage_special.rs` reads imported supplied1453 MAGIC_TYPE8 rules. Same-zone living party summon, self town return, eligible same-zone party town return and20m forward Blink save resources/positions in one transaction. |
| Fire/Ice/Lightning Armor75 | Category25 self buffs consume the original stone and last the original duration. The first damaging incoming NPC physical hit consumes the armor and reflects elemental damage through ordinary NPC health/death/loot credit. |
| Vampiric Fire80 | Original type3/direct16 drains enemy player MP and restores half the configured drain to caster HP, capped by maximum HP. Supplied1453 computes the HP return from configured damage even if the target has less MP. NPC targets are refused before payment. |
| Freezing Distance80 | Category22 player freeze uses the original duration, rejects movement and teleports, and expires normally. The original row has speed1. The supplied implementation has no recovered HP-dependent success roll despite the original description; that exact retail probability remains unresolved. |
| Light Shock62 | Category20 ground-area targeting disruption reaches eligible enemy players, blocks R/selected-target attacks and expires normally; self/ground casts and cancellation remain available. |
| Minor Resist Master20 | Category24 ground-area enemy-player debuff applies the original six20percent resistance reductions after flat bonuses, independent of buff iteration order. |
| Lightning damage | Successful direct lightning NPC hits can stun for two seconds. Chance is truncate(50 - lightningResistance/2), bounded0..100percent. Active stun is not extended by another hit; AI stops its visible destination and pauses movement/attacks until expiry. |
| Mana Shield | Existing incoming-NPC absorption now caps absorbed HP by available MP divided by MP cost;15percent absorption costs4MP per absorbed HP. Zero MP cannot provide free absorption. |
| Friendly buffs | Original Mage resistance and Frozen Armor/Shell/Barrier categories8/2 support a living same-nation player within original range as well as the existing self path. Full party-wide buff distribution remains separate. |

Player resource/control effects operate only on eligible living enemy players
in Colony Zone71, Ardream72 or arena48. Party members and protected admin targets
are excluded. Freezing Distance, Light Shock and Minor Resist retain their
original descriptions' explicit exclusion of monsters. These bounded effects do
not enable ordinary player weapon/spell damage, full PvP or war controllers.
Other sessions reload committed state before handling subsequent requests;
resource/attribute notifications use the current session rather than queued
older HP/MP snapshots. Invalid targets, duplicate categories, cooldowns, missing
reagents and persistence failures roll back payment and all target changes.

Transport uses supplied1453 WarpType1/12/20 and original range fields rather
than later hardcoded replacements. Blink derives heading from accepted movement,
never client destination coordinates, and starts at the forecast's elapsed
current position. Missing heading fails before payment, matching the supplied
source's old-position orientation limitation. Imported original GTD extents
reject out-of-map endpoints; unsupported/missing terrain headers fail closed.
World collision/pathfinding and indoor Blink bounds remain incomplete. Gate and
Escape use the zone's configured start fallback: persistent selected bind points
are not implemented. Escape above zone31 and transport in Forgotten Temple55
are refused following the supplied source; frozen members are excluded.

Primary evidence: original `Skill_Magic_Main_us.tbl`, type1/3/4 tables, Zones.tbl
and GTD headers; supplied1453 `GameServer/MagicInstance.cpp` type3/direct16,
type8 and ReflectDamage, `GameServer/MagicProcess.cpp` categories20/22/24/25,
and `AIServer/Npc.cpp::RecvAttackReq`/FAINTING_TIME. Open-KO1298/9
`src/Server/AIServer/Npc.cpp` corroborates the same two-second resistance formula
specifically for lightning; the primary customization applies it more broadly.
Only lightning is added here; Fire damage is not turned into a stun.

Reflection intentionally uses the computed expression
`((230 * damage / (resistance + 250)) / 100) * 25`, retaining integer rounding.
The supplied source computes that value but erroneously applies full `damage`;
it also derives the element from a group modulo inconsistent with original
Mage groups. The local implementation uses branches5/6/7 for fire/ice/lightning
and the computed reflection value. This is a documented source correction,
not proof of untouched retail reflection balance. Reflection currently uses
actual incoming HP damage; player and monster-spell reflection remain separate.

Absolute Power, Instantly Magic, Bright Dew regeneration and
Absoluteness/Matchless damage reduction already work and are retained. Ice
category6 NPC slow continues to use original duration/speed; full retail slow
success/resistance fidelity is unresolved. Internal branch9 `whipping` remains
unsupported and is not treated as a missing visible skill-tree button.

The extended socket/SQLite regressions cover both nations' level80 fire,
three armor casts/stone use, Blink direction/boundary validation, player-only
resource/control targets and expiry. Other behavioral regressions cover party
transport persistence/refusal, NPC stun/expiry, one-shot reflection and Mana
Shield resource bounds. These do not establish complete live QA of all332
records. Generated per-record reports/raw inputs remain local under ignored
`logs/`, outside Git.

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

Rogue5/10percent drains use the target's **current** HP, with integer truncation,
matching supplied1453 `MagicInstance::ExecuteType3` direct type9 below100.
The server keeps32-bit actual damage and credit, so32767 does not cap these
percentages. The native NPC HP packet has full32-bit maximum/current HP but a
signed16-bit display delta; that field alone is bounded. Large drains also send
a system notice reporting the full damage. Ordinary physical damage caps and
other spell/recovery mechanics are unchanged.


Basic R starts from Open-KO PlayerMySelf::AttackableDistance:
weapon reach + (player radius + target radius)/2. At the owner's request, multiply
that sum by1.20 and apply a3m minimum, then add0.15m packet-rounding tolerance.
Model radii are recovered
from original NPC_Looks/UPC_DefaultLooks joint assets using the frame-zero
joint transform bounds, matching N3Chr::Init/FindMinMax/Radius, N3Joint::Tick/
ReCalcMatrix and N3AnimKey::Load. The target radius is multiplied by packet
spawn size/100, matching GameProcMain's NPC ScaleSet. Skeleton parsing bounds
names, key counts and hierarchy depth; invalid/unavailable radii are logged and
retain the previous2m body fallback. No reference assets are modified or stored
in Git. A0.15m tolerance covers the two positions' decimetre packet rounding.
The server keeps its horizontal distance check with the bounded moving-melee
forecast handling below. Type6 player forms use their original model and size
in place of the human race radius. This change
applies equally to ordinary/admin basic attacks, not to skill-range formulas.
Troll King appearance1701/scale240 has about6.41m reach with a1m dagger on race2,
before rounding tolerance. Small targets still allow at least3.15m.
Regression compares the same skeleton at sizes100/240 and rejects targets
beyond their respective reach, plus animation keys and malformed skeletons.

### Moving melee and false misses

Native-client testing exposed a separate stale final-stop failure after the
forecast changes. On an Orc Watcher, authoritative player coordinates were
(80.0,389.2)m while live client memory stayed at(79.6715,387.9647)m. Both sides
agreed on the target at(78.8,385.8)m. The server consequently checked3.61m while
the client checked2.33m; repeated stationary R packets were rejected. This was
an unreported final player displacement, not a target-radius mismatch. Its exact
client frame-order cause is unconfirmed.

The isolated client now synchronizes a changed position through native MOVE
before sending R; see client-startup. Server movement validation, model reach,
attack cooldown, damage and resource rules remain unchanged. The owner reports
R appears repaired, corroborated by successful post-fix Orc Watcher and other
NPC hits. Broader LAN/latency coverage remains open; unpatched original clients
do not receive this isolated-profile correction.

Native moving06 packets describe NextPos(PACKET_INTERVAL_MOVE), while stop
packets report the current position. Open-KO GameProcMain::MsgSend_Move and
GameDef.h corroborate the1.5-second forecast interval; original captured movement
shows the same lead. At Light Feet speed, treating that endpoint as the current
position rejected a near-target Spike with a13.9–15.8m apparent distance.

After movement passes the existing speed/time/distance envelopes, retain a
bounded segment ending at that report. Its trailing length cannot exceed the
reported allowed speed times1.5seconds. Advance its starting point with elapsed
time and choose the closest point to the target only for basic R and the current
melee-skill path. After1.5seconds, on stop, zone change or mismatched report, use
the ordinary reported position. Sideways distance is not expanded; rejected
movement cannot replace the segment. Persistence, visibility, loot, projectiles,
skill ranges, damage and payment-before-hit rules retain their existing rules.
AI observation uses the elapsed current point instead of chasing the player
forecast endpoint. This is a
documented compatibility inference for native forecasts, not a recovered full
retail movement simulation. Socket/SQLite Spike108655 and R regressions cover
moving hits, stopped/sideways refusal and no resource charge on refusal.

Current basic NPC attacks do not roll a random hit/evasion miss: result0 also
represents range/target/cooldown refusal. The reported R capture shows repeated range
refusals on the same target. Native00479BD7..00479CA0 subtracts half the sum
of radii before serializing distance at0069E550, so its reported0.9m is a surface
distance and cannot be directly compared with a6.83m center distance. Do not
trust that client-provided distance to bypass server reach. See the NPC
movement notes in world-services. No new accuracy multiplier is applied. Supplied1453
Unit::GetHitRate has probabilistic tiers; complete retail accuracy sampling
remains separate from the current physical damage path.

Class/weapon coefficients come from the local coefficient export. Physical
hits use source-backed attack/armor/hit/random paths and original skill values.
Equipped Item_Ext columns30..34 contribute STR/STA/DEX/INT/CHA (magic power),
35/36 add direct HP/MP,37..42 add the six resistances, and10/11 add accuracy/
evasion percentages. These are zero-based original1453 columns, corroborated
by Open-KO's __TABLE_ITEM_EXT and supplied1453 User::SetSlotItemValue.
Item_Ext poison/curse are reordered to the native curse/poison packet fields.
Primary bonuses stack with timed stat buffs before AP, HP, MP, weight and
hit/evasion calculations. Direct HP/MP bonuses are added before existing caps;
item stat bonuses remain at zero durability, while existing broken-item
damage/armor handling remains unchanged. Supplied1453 SetMaxHp/SetMaxMp and
SetUserAbility corroborate these contribution paths.

Mage spell and elemental-arrow magic-power reads include effective bonuses
without truncating the total to a byte. Login sends base/bonus byte pairs;
equipment/buff updates send the existing u16 bonus fields. Source MYINFO in
User::SendMyInfo and Open-KO GameProcMain corroborate these layouts.
Weapon-defense reductions, elemental weapon effects, drains and full PvP
equipment behavior remain incomplete. Imported NPC stats and timed AC/max-HP modifiers affect their damage
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
