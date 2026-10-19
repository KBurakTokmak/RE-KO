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
consumes nothing; failure burns the origin. Accessory/rebirth/sealing are separate
unsupported branches. See [rates](upgrade-rates.md).

## Loot

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

## Travel and scripts

Town via menu and `/town` share source eligibility and persisted travel.
Clickable nation gates use historical SMD warp data; Eslant walkthrough triggers
combine source EVENT actions with terrain tiles. Test movement crossing a trigger
as well as stationary endpoints. Death returns to the source town and clears
relevant effects; spell resurrection and death XP penalties are separate work.

NPC script execution validates identity, proximity, life, nation, class, helper,
quest state and server-offered choices. The restricted Lua bridge exposes only
supported queries/actions, with4MiB and200000-instruction limits. Unsupported
actions roll back all item/coin/quest changes. Ordinary quests use1453 database
requirements/rewards; supplementary Lua is not authority for newer rewards.
Opening dialogs does not establish every specialist NPC service is implemented.
