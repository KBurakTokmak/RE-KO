# World, inventory and NPC services

## Population, visibility and AI

Offline imports use supplied1453 K_NPCPOS/K_NPC/K_MONSTER and original client
appearance tables. The reference roster has2445groups/6568instances across25
zones; old Moradon has59NPCs and474monsters. Content is local imported data,
not bundled assets. Separate training fixtures are retained by imports.

Source spawn rectangles, counts, respawn timers, movement modes and conditional
metadata persist in SQLite. RegenType0 uses the source timer; type1 is an initial
one-life spawn. Unsupported event conditions await controllers instead of guessed
schedules. Dungeon-family metadata alone does not suppress initial spawns.

Rare bosses use `world_boss_pools` and `world_boss_members` over the existing
source spawn rectangles: one living boss per name/zone, choosing a random source
location. Current imports define23pools/122candidate entities in home castles,
Eslant and old Moradon: Talos, Harpy Queen, Deruvish founder, Snake Queen, attila,
Kekurikekukaka, Hyde, Antares, Shaula, Samma, Troll King and Dragon tooth where
source rows exist. Faction zones have independent pools. No missing locations
are invented. Death by direct damage or DoT starts a six-hour timer; ordinary
loot/XP/quest credit is unchanged. Every server start spawns all configured bosses
and resets timers; cooldowns are memory-only. Manual admin summons remain separate.

Evidence: supplied1453 AIServer `ServerDlg.cpp` creates each K_NPCPOS row/ordinal
and assigns `sRegTime * SECOND`; its `Npc.cpp` regen fallback is overwritten.
The supplied data has multiple candidates alive independently and mixed four-hour/
32000-second timers. Grouping, six hours after death and restart resets are
explicit owner requirements overriding those customizations.

Visibility uses48m regions and a3x3 view, native1C lists and1D requested details.
Requests are bounded, deduplicated and restricted to nearby living entities.
Cross-region movement immediately updates visibility. Client movement packets
include predicted endpoints: account for prediction leads without treating them
as actual instantaneous travel or dropping valid direction changes.

AI uses imported aggression, damage, reach, speeds, search/leash and stand time.
Act types1/2 remain passive until attacked. Damage contributors can trigger
retaliation beyond the smaller proximity-search radius, subject to the source
reaction guard. Fresh damage renews pursuit; escaping without attacking permits
return. Threat reset is separate from XP/loot damage contribution history.
Native1453 NPC_MOVE dispatch006A7DD0 calls MoveTo00477F10. That method sets
rendered speed to the distance to its destination times0.85/1.5, corroborating
Open-KO PlayerNPC::MoveTo/Tick. Sending only each250ms simulated step as the
client destination caused accumulating visual lag and false melee range failures.
Movement now sends a destination ahead by speed*1.5/0.85 along the current
heading while keeping authority at the simulated position. Arrival sends the
actual stop immediately; entering attack range, losing pursuit and finishing
return also cancel the previous destination. A future destination may extend
past a planned stop; it is replaced by that stop on arrival, before the client
continues past it. Coordinates remain within protocol bounds. This is a local
synchronization adaptation to the existing250ms simulation, not recovered retail
AIServer scheduling. Native-interpolation regression covers walk/run speeds,
arrival, basic-melee visual distance and no overshoot beyond packet rounding.
Player observations use the validated forecast's elapsed current point rather
than making monsters chase a position up to1.5seconds in the future.

Movement1/2 performs bounded idle roaming;0/4 do not. Terrain pathfinding,
movement3 patrol, family assistance and monster spells remain incomplete.

Complete character assets take precedence over the table joint/animation fields,
following Open-KO `CPlayerBase::InitChr` and `CN3Chr::Load`. The radius reader
resolves the root skeleton from original NPC_Looks column15 (`.n3chr`) when
present. Felankor appearance6200 has reversed table joint/animation filenames;
its valid `Chr/mon_dragon.n3chr` names `Chr/mon_dragon.n3joint`. Reading that root
restores the original model radius and admin summoning without modifying assets.
Malformed, missing or unsafe character paths remain rejected.

## Inventory and services

Item attachment uses original ItemOrg column12. Placement handles both hands,
two-handed conflicts, pants and both rings, preserving displaced items
atomically. Primary stat, direct HP/MP, resistance and accuracy/evasion bonuses
feed effective attributes; see [combat](skills-combat.md). Incoming equipment,
including a bag item displaced into an equipment slot, validates original race,
class family/subclass, base allocated stats and rank/title requirements. Level
bounds come from imported supplied1453 ITEM ReqLevel/ReqLevelMax. Existing
equipment is not automatically removed; slot mapping and displacement are unchanged.

Evidence: Open-KO `UIInventory.cpp::IsValidRaceAndClass` checks ItemOrg13/14
and stats30..34 plus conditional Item_Ext48..52; bonuses do not satisfy base-stat
requirements. Rank/title use ItemOrg28/29 plus Item_Ext46/47, against currently
modeled zero rank/title. Supplied1453 `ItemHandler.cpp::ItemEquipAvailable` adds
server ITEM level bounds. Native UI does not enforce ItemOrg level on ordinary
equipment: the starting dagger has client level8 but server level1, so client
level is not substituted for missing server rules. Other special weapon effects
remain incomplete.
Respect native UI ownership when sending inventory acknowledgements; see
[protocol](protocol.md). Delete-item success/failure must release client waits.

Shops support buying/selling and source stock. Sundries exposes repairs; fees
and tariff initialization use the verified transaction flow. Innkeepers expose
account-shared storage:192slots, eight pages of24, with validated deposit,
withdrawal, stack/bag moves and bank coins. All transactions check ownership,
quantity, range, capacity, weight and applicable coin caps before commit.

Type25 mantle clerks, including Habice, open the native cape window with3C1B
instead of searching for a QUEST_HELPER row. Supplied1453 NPCHandler::NpcEvent
and native006B89AC agree. Source NPCHandler::HandleCapeChange requires a
promoted clan leader. Native70 purchases now validate the nearby service,
leadership, original Cloak.tbl grade/price and wallet before atomically saving
the cape. Training clans receive error-1 without a charge. Knight promotion,
key pickup and persistent capes are described in [social systems](social.md).

The Moradon anvil is **map object5001**, kind24, at approximately337.888,389.948,
not Heppa's character model. Original OPD event5001/type8/status1 and supplied
K_OBJECTPOS agree. Its special-object spawn must include status1 to reveal the
native shape. Resolve an object-click ID0 only to the matching nearby anvil;
reject forged nonzero IDs. Normal/preview upgrades match source recipes,
reagent multisets, item classes and valid original-client result items. Preview
consumes nothing; failure burns the origin. Reverse Scroll conversion uses source
recipes plus the owner-requested plain-weapon compatibility rule. Accessories,
sealing and remaining specialist branches are separate. See [rates](upgrade-rates.md).

## Loot

`/autoloot` toggles automatic pickup for the current login; `/autoloot on` and
`/autoloot off` set it explicitly. It starts off on reconnect. Plain `autoloot`
also works if a client intercepts unknown slash commands. The isolated verified
client profile forwards this exact slash command through ordinary chat; other
native slash commands retain their original dispatch.

Once per second, up to32 nearby owned bundle entries pass through the same
transactional pickup/party-routing path as manual loot. Existing life, zone,
approximately15.56m distance, ownership, coin cap, capacity, weight and stack
checks remain authoritative. Failed/full-bag pickups leave the drop available;
there is no direct grant or repeated payout. Pickup pauses while near an active
NPC service/anvil context to avoid changing icons owned by that window; moving
away resumes it. The command and its status reply are private.

Supplied AIServer/Npc.cpp GiveNpcHaveItem/ItemProdution and GameServer delivery
handlers are the reference. Each death generates once; the largest credited
direct/DoT damage contributor owns the bundle. Training fixtures do not drop loot.

- Coins are random70..100percent of iMoney; values>=32767 become32000.
- Five independent1..10000 rolls use imported source chances. First two slots
  select among30-entry groups, including empty entries; low group IDs support
  MAKE_* procedural generation. Final three slots use direct item IDs.
- Direct arrows in391010000..392010000 drop20; group items drop one.
- Missing original-client items retain their no-drop probability; no rerolls or
  invented replacements. Conflicting duplicate source keys retain the first row.
- Pickup validates owner, zone, life, distance, stack/capacity, weight and coins
  atomically. Replays/full-bag failures preserve the bundle.

Bundles are memory state and disappear on daemon restart. Exact nonempty-bundle
expiry is unrecovered; do not invent a timer. Party distribution is described in [social systems](social.md).

### Suppression quest loot repair

The supplied1453 lunar1453.bak, full backup position2, has empty K_MONSTER_ITEM
rows for7005/7007/7040, missing7006/7008 rows, and similarly incomplete7010..7014.
SQLite correctly imported those omissions; this was not a random-roll or client
item-filter bug. The supplementary2086 export uses newer810418000 drops and is
not suitable for the original1453 suppression quests.

`scripts/Repair-1453SuppressionLoot.py` is an explicit offline repair, not a
startup grant. It validates original client items, quest exchanges94/98 and
zone/prototype/drop links. Normal zone82 mobs7005/7006/7007/7009/7040 drop
Certificate of Hunting910138000; Grape Ripper7008 drops910135000. Zone83 mobs
7010/7011/7012/7014 drop910137000; Patos's messenger7013 drops910136000.
Each certificate occupies direct slot2 with chance10000. This guaranteed local
repair uses the existing1453 certificate-drop pattern (7000/7001) to unblock
progress; the missing monsters' exact retail rates have not been recovered.
Gates do not receive loot, and no certificate is inserted into a character at
startup or on script execution. Normal corpse ownership/pickup rules apply.

The owner approved estimated old defense-armor drops after the empty source
tables were confirmed. [Kalais' first-person MSS guide](https://ko.kalais.net/mss.php)
reports tier5/chitin for MSS II and tier1 for MSS III, all at+1, with dagger
helmets, club pauldrons, sword gloves, axe pads and spear boots. Original1453
ItemOrg and Item_Ext13..17 verify all40 selected class/part variants: suffix541,
helmet/boots defense3 and other parts5. No item stats or client assets are edited.

The guide's aggregate and per-defense estimates are not mathematically
consistent enough to recover a complete table. The approved local approximation
uses independent slot0 mage/priest and slot1 warrior/rogue pools at150/10000 and
100/10000. Each pool contains30 entries; each of its two classes has helmet1,
boots2, pauldron3, pads4 and gloves5 entries. Any armor drops on2.485percent of
kills (expected0.025armor per kill), with a0.015percent chance of two armors.
Any-class dagger helmets are approximately0.1666percent of kills. Class bias
and exact part weights are local choices preserving the guide's qualitative
rarity order, not measured retail probabilities. These pools apply to regular
suppression mobs; bosses receive their quest certificate only.

Group IDs82001/82002/83001/83002 are reserved for this repair. Imports refuse
collisions with nonempty unrelated rules/groups and preserve all other loot,
coins, spawns, characters and quest progress. Rates can be selected for an empty
installation using --light-rate/--heavy-rate; changing existing rates requires
explicit review rather than silently replacing customized data.

## Travel and scripts

Town via menu and `/town` share source eligibility and persisted travel.
Clickable nation gates use historical SMD warp data; Eslant walkthrough triggers
combine source EVENT actions with terrain tiles. Test movement crossing a trigger
as well as stationary endpoints. Death returns to the source town and clears
relevant effects; spell resurrection and death XP penalties are separate work.

`Repair-1453MageTravel.py` repairs bounded missing travel rules offline. Legacy
Colony Zone destination201 is normalized to original1453 zone71 while keeping
the historical name, fee and coordinates. Missing Delos30 groups301..305 come
from Open-KO `siege_0722.smd`, including nation-specific old Moradon exits and
the Abyss32 entrance. Colony Zone's711/712 return groups reuse historical
`freezone_1217.smd`, with IDs adapted from2011/2012 groups. Newer Moradon return
coordinates are not copied into the old client layout.

Ardream72 exists in original Zones.tbl as FreeZone_a and has nation gate objects,
but the supplied1453 backup omits its START_POSITION, population and warp routes.
Incoming routes/fees use supplementary `karus2004.smd`, `elmo2004.smd` and
`moradon_0826.smd` records whose Ardream destinations match the original nation
gate geometry. Supplementary `freezone_a.smd` supplies exit groups721/722; its
Moradon coordinates are replaced by the compatible historical Delos exits.
The explicit town fallback uses the incoming nation destinations(843,140) and
(190,897), with provenance distinguishing this adaptation from primary1453 data.
Original GTD extents validate outdoor endpoints. This enables travel, not a
claim of a complete Ardream NPC/monster roster; missing spawns are not invented.

Gate menus show both combat zones; selection requires level35+, positive NP,
and a maximum of59 for Ardream. Administrative characters bypass those entry
limits, matching supplied1453 CharacterMovementHandler::CanChangeZone. Fees and
position changes commit atomically after proximity/nation/offered-ID checks;
replayed selections cannot charge again. Full PvP, war closures and siege
controllers remain separate. The repair preserves all unrelated rule records,
character tables, loot, items and quest data and refuses occupied destination
slots instead of silently overwriting custom routes. It is idempotent.

NPC script execution validates identity, proximity, life, nation, class, helper,
quest state and server-offered choices. The restricted Lua bridge exposes only
supported queries/actions, with4MiB and200000-instruction limits. Unsupported
actions roll back all item/coin/quest changes. Ordinary quests use1453 database
requirements/rewards; supplementary Lua is not authority for newer rewards.
Opening dialogs does not establish every specialist NPC service is implemented.
