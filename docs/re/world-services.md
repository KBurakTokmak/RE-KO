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
location. Current imports define37pools/185candidate entities in home castles,
Eslant, old Moradon and Colony Zone: Talos, Harpy Queen, Deruvish founder, Snake Queen, attila,
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

Colony Zone71 adds14 pools over63 existing candidate instances: Lobo, Lupus,
Lycaon, Barrkk, Barkirra, Lesath, Shaula, bone collecter, Duke, Bishop, Bach,
Javana, Samma and Orc bandit leader. Both leader prototypes share one name pool.
The imported primary timer is16000seconds (4h26m40s); the existing owner policy
uses21600seconds after death instead. The source candidates remain in place,
with only one selected per pool. Riote, Atross, Blood Don, Undying, bandit
officers and other ordinary groups are not deduplicated or retimed. Existing
Ghost of Manes/Odin event conditions are unchanged.

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

Owner-defined economy defaults live in `config/item-balance.json` and migration
`007-potion-balance`, persisted as ITEM_ECONOMY rules. The ten shop HP/MP potions
have half their original weight. Seven cost20percent of original price; Water of
favors, Potion of wisdom and Potion of soul retain75percent (5250,5250,11250Noah).
Speed+ Potion800015000 costs1000Noah at Sundries stock, including Moradon; its
weight stays unchanged. Prices are absolute, so restart/reimport does not compound
discounts. The shared item catalog applies overrides to buying, resale and every
server bag-weight/capacity calculation, including already-owned items. Existing
balances, quantities, drops, skill effects and ordinary creation grants are unchanged.

These overrides are **server-side only** by owner choice. Original client tables
retain their prices/per-item weight text and may enforce old local affordability
or weight checks before sending a purchase request (Open-KO
`UITransactionDlg.cpp::ItemCountOK`). Server replies use the new actual balance/weight.
Do not alter client tables or falsify server balances to conceal that limitation.
Storage preserves the count of an existing item stack even when the original
client table marks that scroll nonstackable. It may move into an empty slot,
but separate nonstackable stacks cannot merge. This matches supplied1453
`ItemHandler.cpp::WarehouseProcess`; the captured Magic Shield Scroll request
deposits18 items. No item-table or character-data rewrite is needed.
Native money transfers use page/source/destination255 sentinels. They bypass
item-slot validation and retain the same atomic ownership, balance and coin-cap
checks. Opcode45 requests and refusals are captured; successful transfers log
the action and character balance before/after for debugging.

Delos uses the existing original1453 K_OBJECTPOS gate groups301..305 and the
five destinations from Open-KO `siege_0722.smd`: nation-specific Moradon returns
and the neutral20,000Noah Abyss entrance. Database migration `001-delos-travel`
adds absent routes, validates matching gate nations/destination starts and refuses
conflicting local records. See [database setup](../setup.md).

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

### Chaotic Generator and Hemes

The imported Moradon603/type162 Chaotic Generator opens native5B/4 with its
instance ID;5B/5 exchanges one owned fragment389160000..389166000. The original
request is9bytes: opcode/subcode, NPC:u16, fragment:u32, selected bag slot:u8.
Native004B4106..004B4130 and a captured slot5 request confirm the final byte;
the supplied primary C++ reader ignores it, which previously led to an incorrect
8-byte server guard. Validate the selected slot's ID/count and decrement that
exact stack, including when another identical fragment exists earlier in the
bag. Never substitute the first matching stack. The result echoes the actual
source position; malformed/trailing requests and empty/foreign slots fail without
mutation. Opening a
nearby generator, living/entered state, exact packet length, supported item,
inventory capacity and weight are validated. Payment and one original-table
reward commit together. Missing inputs, a foreign/distant NPC, full inventory,
invalid reward data or SQLite failure leave all items unchanged. A final fragment
may free its own slot; inventory replacement uses the existing remove/create
icon update before the native result/effect. Repeated requests each require a
remaining fragment. For countable rewards, fill the first matching stack below
9999 before choosing an empty slot, even when consuming the last fragment frees
an earlier slot. This follows primary1453 User.cpp::FindSlotForItem/GiveItem;
equipment is never merged and a full stack remains unchanged. The result's
destination is the actual updated stack, not the primary BifrostPieceProcess's
premature GetEmptySlot result. Regression coverage includes final/nonfinal
fragments, full/near-full stacks, existing durability and nonstackable equipment.
Anvil/reverse-upgrade paths remain separate.

The primary ITEM_EXCHANGE101 rows contain one weighted reward each; all rows
for each of the seven fragments sum to10000. These complete per-fragment weights
are sampled directly, preserving every imported item and count-as-weight. Counts
are not quantities: success gives one item. The supplied1453 BifrostPieceProcess
instead selects a row uniformly before an underfilled per-row weight array,
which effectively flattens different row weights. The direct10000-weight draw
corrects that inconsistency rather than claiming the custom C++ RNG is retail.
Native open/result/object9 effect layouts follow NPCHandler::NpcEvent,
User::SendAnvilRequest and UpgradeHandler::BifrostPieceProcess. Socket/SQLite
tests cover every fragment pool, native frames, slot reuse, replay after the
last fragment, distance, capacity and forced transaction failure. A regression
replays the exact captured9-byte request with duplicate stacks, validates source5
payment and rejects missing/trailing bytes, slot28, an empty slot and an item-ID
mismatch without changing either stack.

Peddler16085/type132 now uses a bounded1453-compatible menu in hemes.rs.
The supplementary16085_Hamus.lua root menu references later labels2013/2014/
2022/2015/40645/4130/4404/8656, all absent from original Quest_Menu_us.tbl.
The replacement exposes the verified Valkyrie Armor/Helmet labels4286/4287,
four bonus choices each, original English confirmation dialogs, Back and Close.
One owned armor800180000 or helmet800170000 certificate is exchanged for the
source variants508011441..444 or508013318..321. Existing Lua transaction,
proximity/offered-choice/replay, weight/capacity and icon updates are retained.
All eight output item/extension rows exist in the original client. The later
Lua's timed30-day grants are deliberately not used: the owner requires permanent
costumes. The eight redeemed Valkyrie variants now equip in the native auxiliary
helmet/armour slots, persist per character in character_costumes and reload on
login. Ordinary equipment remains separate. Owned single items can swap with a
matching costume or return to inventory; unequip into the bag before deleting.
Bag changes, costume changes and clamped vitals commit together before success;
failed ownership/slot/eligibility checks or SQLite errors leave them unchanged.

| Choice | Armour508011441..444 | Helmet508013318..321 |
| --- | --- | --- |
| HP | +200 | +100 |
| Defence | +30 | +20 |
| Monster EXP | +4% | +2% |
| Monster Noah | +6% | +3% |

One choice applies per piece; the two pieces stack additively. Only equipped
costumes contribute. HP/defence enter effective attributes before percentage
buffs/passives; equipping does not heal and removing HP equipment clamps current
HP. Costume durability does not wear through ordinary armour/weapon combat.
EXP applies to positive monster credit after damage/party/level sharing; quest
bonus rewards and NP remain unchanged. Noah applies once to monster coin pickup,
per recipient after party splitting, with existing ownership and wallet caps.
Bank transfers, sales and quest coins do not acquire this bonus.

Evidence: original English Quest_Talk4903..4910 provides the exact bonus values,
and ItemOrg/Item_Ext13/15 supplies the eight IDs and attachment105/107. Native
004BE6FF..004BE89D emits1F directions7/8;004C1540 accepts slots1/4 and original
004C0727 accepts universal costume race20. Native006A0F43..006A1042 loads the two
auxiliary login records. Other-player appearance006A443D..006A44B4 reads upper/
hair costume records after ordinary equipment, followed by zone. Open-KO1298
ordinary inventory has no matching costume lifecycle; supplied1453 ItemHandler
corroborates directions/slots and User::ApplySetItemBonuses corroborates additive
HP/AC/XP/coin handling. Its later extra costume/bag slots are not imported.

Future owner-controlled change: timed costumes are deferred. Keep existing items
permanent unless the owner explicitly requests conversion. A timed implementation
must define activation versus acquisition time, add per-instance expiry metadata
that survives inventory/bank moves, verify native timer fields, expire bonuses/
appearance atomically online and on login, and migrate existing permanent items
under an explicit policy. No hidden30-day timer or inactive timed-mode switch is
present. Other later Peddler services remain outside this bounded implementation.

KC Hemes27011 references27011_KC.lua, which is absent from the supplied packages.
The owner prefers existing Power-Up Store access over adding a KC economy.
Its local service therefore opens the already supported Sundries255000 catalog,
with existing coin prices and atomic shop transactions. No cash balance, unlimited
item grant, new price list or external purchase flow is added. The NPC's imported
appearance/location remain intact; only its runtime service kind/group is adapted.

### Town and zone gates

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
Original GTD extents validate outdoor endpoints.

`Repair-1453CombatZones.py` separately repairs the absent Ardream population:
379 monsters and63 NPCs/towers. Its supplementary2086 K_NPCPOS centers, counts,
directions and timers are adapted to original1453 FreeZone_a terrain, not claimed
as a recovered retail1453 spawn layout. Monster spawn ranges use the supplied
iRange as a square around each center; stationary NPCs keep exact centers.
All bounds and appearances are checked against the original client. Every
monster uses an existing primary1453 MONSTER_TEMPLATE, AI and loot rule rather
than importing newer combat/reward statistics. Treant8851/Ancient8854 map by
matching name to primary5401/5301; towers31403..31406 map to primary5300/5310/
5400/5410 with the same model, nation and level. The absent Zamed24425 static
NPC template comes from the supplementary package and uses its original-client
appearance; its quest implementation remains separate. Later Chaos Stones,
rental booths and the Ardream event monument (seven instances) are excluded.
Potion, sundries, storage and tower visibility at both towns and live monster
streaming are covered by socket tests. This is bounded population restoration,
not a claim of complete tower combat, PvP or all Ardream quest/event controllers.

The repair is offline, checks collisions/customizations, is idempotent, backs up
the save and verifies hashes for all unrelated tables. It changes only population,
population conditions, boss pools/members and the Bifrost policy rule. Run
`python scripts/Test-1453CombatZones.py` to verify a disposable copy and provision
`logs/combat-zones-test.db` for the Rust population/scheduler/socket regression.

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

### Bifrost gate and return

Colony Zone's imported NPC602/type150 uses QUEST_HELPER4260 and the supplied
`602_Bifmove.lua` menu. Its CheckBeefRoastVictory and ZoneChange bindings are
enabled only for that validated nearby gate. `16094_Asan.lua`/helper4243 gets
the same bounded travel binding for the two imported Bifrost return gatekeepers.
NPC identity, zone, life, proximity, script filename and offered choice are
revalidated on every reply; travel commits before the zone-change packet and
clears the dialog, preventing replay. Frozen characters cannot travel.

The explicit current owner requirement is **always open to both nations**.
`world_reference_rules/BIFROST_ACCESS/31` stores `mode=always_open`; the scoped
victory query returns3 only in that mode. Missing/unknown policy fails closed.
Entry coordinates(78,730)/(245,950) match the primary START_POSITION row and
supplementary Lua. Asan retains the source's level30 branch: lower levels return
to their home castle, otherwise to nation-specific Colony Zone center positions.
No items, coins or quest state are charged. Cross-zone population waits for
the existing client loading handshake. Original models, spawn data and loot
inside Bifrost are unchanged.
The supplementary Asan dialog959 is absent from the original1453 talk table;
only that scoped prompt maps to original English4071, "Do you want to leave?".
Entry talk4470/4471 and menu4200/4199, and exit menu4071/4072 exist in the
original tables. No original client text or assets are modified.

The always-open policy is a local override, not an implemented scheduled event.
**When the owner asks to switch to monument/event operation, implement and test
the event controller before changing this rule.** Setting `mode=event` today
closes entry; it does not activate an event scheduler. Keep Asan exit available
when entry closes. The supplied1453 GameServer EventHandler::BifrostProcess and
GameServerDlg::Timer_BifrostTime describe monument victory, the two-hour winning
nation window and the other nation's final hour, but that timer thread is
commented out in the supplied package. Exact retail1453 schedule remains
unverified. The owner's future chat instruction authorizes a subsequent change;
no new in-game event administration command is currently claimed.

### Abyss Dungeon teleporters

The37 imported NPC7001/type28 instances in zones32/33/34 use the shared
`7001_dungeun.lua` script through QUEST_HELPER4056. They are not object warp
gates, so adding HISTORICAL_WARP rows does not repair them. The script requires
GetEventTrigger, ZoneChange, ZoneChangeParty and RobAllItemParty; these bindings
are now enabled only for that Abyss script and matching living, nearby NPCs.
Other NPC scripts retain their existing restricted action set.

`abyss.rs` resolves the clicked instance's persisted TrapNumber through the
primary1453 EVENT_TRIGGER table. The source ranges are40001..40020 for
Desperation Abyss32,42001..42016 for Hell Abyss33, and43601 for Dragon Cave34.
Missing, ambiguous, disabled or wrong-zone trigger records fail before script
execution. No runtime data import, quest rewrite or spawn relocation is needed.
The existing100 trigger rules were compared to the supplied1453 backup and match.

Same-zone script and queued party transfers send destination NPC/monster details
and the region ID list immediately after the warp, without waiting for movement.
Cross-zone population still waits for the existing zone-load handshake. This
matches the primary1453 Warp's NpcInOutForMe and older Open-KO Warp's
RegionNpcInfoForMe ordering; socket regression checks both arrival paths.

The supplied supplementary Lua defines forward/back floor coordinates, Delos
exit, Hell entry, four shortcut keys and the Marble of the Dragon connection.
These destinations align with imported primary1453 teleporter positions, and
the corresponding menu/talk IDs exist in the original1453 English client tables.
Travel is limited to zones30/32/33/34 and requires a configured destination
within30m of its source floor teleporter (or the Delos start). No client-supplied
coordinates or direct event IDs are accepted. Menu choices, NPC proximity and
filename are revalidated on every reply; travel clears the dialog context.

Required keys are retained:379080000 for the script's locked floor transitions,
379081000..379084000 for shortcuts,379111000 for Hell entry and389200000 for
Dragon Cave. The script's distinction between caller-only and per-party-member
key charges is preserved. Party inventory removals and all destination saves
commit in the same SQLite transaction; a missing key rolls back everyone.
This also fixes the supplied Lua's unchecked false return from RobAllItemParty:
failed party payment must not permit a free transfer. Remote inventory counts
and transfers are queued only after commit, using existing social delivery.

Local party safety requires every online party member to be alive, unfrozen,
in the same zone and within50m of the caller, without a pending transfer.
The primary C++ ZoneChangeParty instead iterates all online members regardless
of location; the local guard prevents pulling distant players or charging their
keys across unrelated zones. Solo travel remains available. Source shortcut
labels and coordinates are retained rather than inventing additional floors;
exact retail reconciliation of the18th-floor label in the16-floor Hell layout
is unresolved.

References: supplied1453 GameServer/User.cpp::GetEventTrigger,
ItemHandler.cpp::RobAllItemParty, CharacterMovementHandler.cpp::ZoneChangeParty,
QUEST_HELPER/EVENT_TRIGGER/K_NPCPOS; original Quest_Menu_us/Quest_Talk_us tables;
supplementary Quests/7001_dungeun.lua. Older Open-KO User::ClientEvent corroborates
type28 plus TrapNumber event dispatch. Socket/SQLite regressions exercise all37
menus,34 forward transitions, missing keys, shortcuts, back/exit, Hell/Dragon
connections, wrong-zone trigger rejection, party rollback and replay refusal.

Wolf-family loot audit: supplied1453 group180 is shared by500/501/502/550/551/552
Werewolf/Lycan/Loup-garou prototypes in the existing Moradon/home-zone spawns.
The source group has30 entries and a10percent group roll. Two-Handed Sword(+8)
125310048, Bow(+8)160100048 and Burning Wood Staff(+8)180110508 each occupy one
entry (1/300 per kill); Panic Maul(+8)140210048 occupies two (1/150). Original
client base/extensions validate all four, including the staff's508 extension.
The imported pool matches the supplied source; no rate or drop-table repair is
needed. These are reference-package rates, not a claim about every retail server.
