# Quests, rewards and progression

Ordinary kind1 quests use imported1453 QUEST_HELPER, ITEM_EXCHANGE and
QUEST_MONSTER requirements/rewards. States are0/unset,1/accepted,2/completed,
3/ready and4/abandoned. NPC menus and reward choices are scoped to offered
actions. Rewards, costs, items and progression commit atomically; replaying a
completed menu cannot pay again. Specialist quests remain partial.

## Capital quest lists

Luferson's Melverick (24427) and Shymer (24435), and El Morad's Zalk (14427)
and Clark (14435), open the native quest list through their imported 1453
greeting event 165. The later Lua scripts use a different greeting event.
When the client selects a status-255 list entry, the server resolves it to
the matching eligible 1453 offer and uses the standard quest acceptance and
reward handlers. The guards' ordinary offers begin at level 18 and the
merchants' at level 31, subject to the imported experience requirements.

The original client and supplied server both mark rogue exchanges 841
(Melverick) and 845 (Zalk) as type 12. The turn-in opens a native reward
selection dialog (SelectMsg mode 5) for one Kukry +2 or one Crossbow +2.
Ordinary fixed-reward turn-ins retain mode 4.

Verca and Telson's Shadow Seeker Hunts are quests 813 and 814 in the
original helper and exchange tables. Each requires 20 kills of prototypes
503 or 553. The list entries resolve to the class-specific offers. Rogues
choose one Cleaver +3 or Horn Crossbow +3 (exchanges 889 and 893). The other
classes receive their fixed weapon rewards. The original client replies with
the selected reward's menu index in mode 5, including index 1 for the second
weapon. The server offers one action per valid reward slot.

The Skeleton Knight Hunts (835 and 836) require 30 kills. Priest exchanges
1028 and 1032 list Royal Knight Pad (285002292). The item exists in the
supplied 1453 server ITEM export, but the original English client's
`Item_Ext_14_us.tbl` lacks extension 292. With that client catalog, the
owner-approved reward is Priest Chitin Armor Pads +1 (285002001) and the
dialog displays its name in a private chat message. The original item
remains the reward when its extension is present.

## Quest journal commands

In ordinary chat, type `quests` (or `quests 2` for the next page) to privately
list this character's accepted/ready quests, eight per page. `quest <ID>` shows
the original Quest_guide_us.tbl title/accepted description and independent
monster counters. The guide row comes from the client Quest_Helper_us.tbl
guide column (16) for the character's class and nation; guide IDs are not quest
flags. Quests without a matching guide title retain their numeric
ID. Completed, abandoned and internal32000+ flags are excluded.

`questcancel <ID>` abandons an active ordinary quest using the same native
helper cancellation handler, ownership/class/nation validation and persisted
counter reset as the client. Completed quests cannot be reset this way.
Suppression-zone return behavior below applies equally to this command.
Only the selected quest's counters change. Commands and replies are private,
available to ordinary players, and accept an optional leading slash. Use plain
commands if the native client intercepts an unknown slash command.

This extends server access to concurrent quests; it does not add a new client
window. Open-KO's native Quest tab is disabled and its Q binding is GM movement,
so it is not a verified1453 journal shortcut. Clan promotion is a separate
clerk service described in [social systems](social.md).

## Concurrent objectives

The server supports **40 accepted/ready quests**, excluding completed, abandoned
and internal32000+ flags. This is the owner's requested local capacity, not a
proven retail1453 limit. Supplied1453 has one active monster quest and a separate
200-record serialized quest limit; older Open-KO has100saved records.

`character_quest_kills` keys counts by account, slot, quest and group1..4. First
creation migrates the old shared32001..4 counts to the previously selected
active monster quest once, transactionally. Historical reserved rows no longer
drive live counters. Accepting another quest cannot overwrite existing counts;
completion, abandonment and reaccept reset only that quest. Each kill advances
all matching accepted quests independently up to their source requirements.

The native client has one counter context. Select each changed quest using
640901 with its previous counts, then640902 for its increment. NPC dialogs
select the viewed quest; login selects the first active monster quest. Server
progress remains independent across all40 even though this native context is
singular. The owner confirmed multiple quest acceptance works in-game.

Native item checks005F4A10 compare actual bag/wallet requirements and issue
6404/helper when ready or640C/helper when items are lost. Native005F50E0 requests
kill completion readiness. Validate owned persisted objectives remotely and
transition1<->3 without granting rewards, consuming items or opening dialogs.
Reward claims still require normal NPC interaction. Unchanged readiness emits
nothing; appending completed-count snapshots here creates a native callback loop.

Wallet notifications use4A/direction/delta/balance immediately after commit.
Quest XP uses1A00/current XP. Native chat output is conditional: a missing chat
line alone is not proof of a missing reward. The reported Bulcan500XP reward
was verified against client/imported exchange, live packet and saved XP; no
regrant was necessary.

## Monster Suppression Squad entry

Captain Fargo21510 and the matching11510 captain use imported quest603/605
helpers and exchanges. Acceptance commits the active quest and saved destination
in one transaction, then starts the existing27 zone-loading handshake. Incomplete
active quests offer entry again; turn-in still checks the source exchange and
can pay only once. Helper state selection preserves the quest's class/nation.

The supplied1453 rogue exchange95 has bRandomFlag11 and awards both Crossbow
168110045 and Cleaver111010045, one each. Source GameServer/ItemHandler.cpp
RunExchange treats11 as an all-rewards bundle, not a choice. Consuming three
910138000 and one910135000 and completing603 therefore delivers both weapons;
do not regrant/reset a completed quest merely because a choice window was
expected. Exchange12 selection is a separate existing path. Quest item updates
now clear replaced IDs before creating reward icons and mark received items
for native notifications; see [inventory protocol](protocol.md).

Quest603 enters zone82 at202,199, corroborated by supplementary21510_Hwargo.lua
event6095 and the original map's Grape Ripper population. Quest605 enters zone83
at its imported START_POSITION, with the matching Patos messenger population.
No new monsters, quest items or rewards are synthesized. Original1453 talk/menu
IDs and existing inventory capacity/weight checks remain in use.

The owner explicitly allows solo entry. This path transfers the interacting
character; coordinated party-wide transfers are separate. Cancelling603/605
inside their respective zones atomically abandons that quest and saves the
nation's imported home-town position before returning through the same loading
handshake. Supplied1453 QuestHandler::QuestV2PacketProcess case5 corroborates
returning players from suppression zones on cancellation. Other quests and
their independent counters remain intact.

The suppression loot repair supplies the missing certificates through ordinary
corpse bundles, including Grape Ripper's Certificate and the zone83 equivalents.
It does not grant quest completion, change exchanges/rewards or refund cancelled
progress. See [loot evidence and estimated armor rates](world-services.md#suppression-quest-loot-repair).

## Suppression exits and boss completion

The original K_OBJECTPOS portals have groups821/822 in zone82 and831/832 in
zone83, scoped to Karus/El Morad. When no historical warp destination exists,
those exact portals offer a free return to the nation's imported START_POSITION.
Existing native gate-list/selection and27 loading packets retain distance,
nation, offered-ID and persistence checks. Supplementary Map/Map SMD files send
players to newer Moradon815,539 for1000Noah; those mismatched destinations are
not copied. Other zones and existing warp records retain their current behavior.

Natural Grape Ripper7008 and Patos's messenger7013 deaths start a shared-zone
60-second loot grace period. English private-chat countdown notices appear on
entry/reconnect, every10seconds and the last5seconds. This is an explicit local
duration; the exact original1453 completion delay/native timer window is not
established. No unverified event-window packets are sent. Direct and periodic
damage deaths use the same transition; repeated death checks cannot extend it,
and manually summoned copies cannot close a natural run.

At expiry each entered character in the zone saves its home position before
starting the normal zone-loading handshake. Failed saves leave the character
in place for a later retry. Loot, wallets, accepted quests and other counters
are preserved; NPC reward turn-in is still required. Early portal travel clears
the character's local notice context. Reconnecting in the same running server
uses the existing deadline. Expired shared runs refuse new clerk entry.

Destructible Gate7033/7034 deaths also emit native NPC_OUT, removing the dead
barrier immediately rather than leaving its character/collision until corpse
cleanup. The gate's imported HP and one-life regeneration policy remain intact.
These are shared populations, not party instances: one boss ends the shared
zone, and restarting the server resets its populations and completion timer.

## XP, points and starting defaults

All80 imported LEVEL_UP thresholds are checked against original
Player_experience.tbl at startup. Runtime progression caps at80. Solo monster
XP/NP uses ceil(baseReward*contributedDamage/totalDamage), followed for XP by a
second ceiling with the source level-difference multiplier:

| Monster level minus player level | XP multiplier |
| --- | --- |
| <=-14 | 20percent |
| -13..-8 | 50percent |
| -7..-2 | 80percent |
| >=-1 | 100percent |

Reward eligibility requires a living entered character in the same zone within
50m of death. Rewards are separate from loot ownership. XP carries across
thresholds; at80 it caps at that level's threshold. Level-up gives3 stat points
through60,5afterward, and2 skill points per level starting at10; stats/vitals
recalculate. Normal point allocation preserves current HP/MP on max increases.
Death XP loss/delevel/refunds remain unimplemented. Party XP and nearby quest
kill sharing are described in [social systems](social.md).

Normal new characters receive20,000Noah,300NP,0monthlyNP and the configured
original+1 starter gear. The300NP insertion trigger runs at creation and never
refills spent NP on restart. Do not replay historical testing grants or change
normal defaults to match administrative fixtures.

## First job change and skill gates

At level10, [Grand Merchant] Kaishan in old Moradon464,302 offers the first job
change: quest406/exchange460, source fee3000Noah. Beginner classes1/2/3/4 map
to5/7/9/11 within their nation. Commit class, fee and completion once, preserve
earned points/items/XP, then send native3406/class:u16/player:u16 to refresh
skill UI (006C5450). Verified native menu text IDs are4064prompt,4062confirm,
4063decline,4065fee failure,4068already,4069low level,4066/4067nation success.

High-level skill gates use persisted completed quest flags
(`Skill_Magic_Main_us.tbl` column 29). Admin fixture unlocks stay separate.

## Master promotion and skill-unlock chains

Quest 408 (level 60) runs at each nation's class master: Skaki, Clarence, Drake
and Minerva (24201-24204 in Luferson, 14201-14204 in El Morad). Exchanges
461-464 take the class items. Turn-in follows known 1453 server
`PromoteUser` behavior in one transaction:

- Novice 5/7/9/11 becomes master 6/8/10/12 in the same nation.
- Base-class flag 1-4 and quest 408 are saved as completed.
- The native class packet is `34 06 class:u16 player:u16`, sent to the player
  and to visible players in the region.

The job-change (900006000) and skill (900007000) rewards are markers, as in 1453
`RunExchange`, and never enter the bag. A 408 offer needs a novice. Offers whose
exchange unlocks skills (51-54, 510-523) need a master. Quests already held
stay cancellable and completable. Master skill points
(branch 8) open after promotion.

Ingredient quests 114-118, 214-218 and 307-318 are repeatable. Each turn-in
returns the quest to the re-offer state (4), so Magic Jewel Powder (quest
118/218, exchange 41) and the stamps, nostrums and scrolls can be crafted as
often as the skill quests consume them. Supplementary 2086 scripts save state 4
for these quests. 114/214 (Armband) and 316 (Holy Water of Poison) are a local
rule: 1453 exchanges consume their products in both 408 and a skill quest.
Migration 012 reopens these quests on saves that completed them under the old
one-shot rule.

## Native list entries and generic quest steps

The native client sends the status-255 list entry helper in `64 03`/`64 06`
(confirmed by captured requests). For every data-synthesized quest, the server
maps that entry, or a helper whose status no longer matches the save, to the
same NPC and flag helper for the saved state: offer (0/4), active (1/3) or done
(2). Other NPCs keep their reference scripts.

- **Accept-side items:** accepting quest 8 gives the Reconnaissance Order
  (1453 exchange 87 has no inputs), quest 9 the Order of seven key and quest 430
  the Book of Suffering (supplementary 2086 scripts). An item already held is
  not duplicated.
- **Exchange steps (type 3):** the seven key sentinels (416-422), the suffering
  watchers (431-437) and the Recons (137/237) use their reference scripts with the
  bound `isRoomForItem`. The level-50 chain is: accept 9, swap keys 1 to 7, then
  hand the Seventh key to Fargo or Falkwine (15,218,948 XP).
- **PvP objectives:** quests 438/439/442/443 count opposing-nation player kills
  (QUEST_MONSTER target 1 or 2 is the victim's nation; `900008000` is the
  objective). Direct PvP kills count. Arena deaths do not count, as in 1453.
  Damage-over-time kills do not count yet.
- **Bound script functions:** `PromoteUser` (only for the 408 helper), `isRoomForItem`,
  `CheckSkillPoint`, `RunSelectExchange` (runs a fixed 1453 exchange),
  `CheckLoyalty`, and stubs for `CheckKnight` and `CheckClanGrade`.
- **Quest 883:** the data names nonexistent prototype 1551. Its Karus twin, quest
  882, uses the spawned LICH 4551, so migration 012 corrects the target.

`quest_walkthrough_tests.rs` walks every synthesized quest variant from the
native list entry through acceptance, kills, items, turn-in, rewards and replay.
It also covers the four class chains from novice to every skill quest, crafted
only from raw drops, plus the key, suffering, reconnaissance and PvP chains.

## Remaining quest gaps

These need evidence or an owner decision before they can be implemented:

- **No item source in 1453 data:** Teeth of Bandicoot (102), El Morad Symbol
  (69), Secret Scrolls (63/64), Proof of Orc Bandit Boss (600), Black silk spool
  (125/225), Rotten Eye (564), Ork Talisman (569) and Prayer of favors (412).
- **Moira's treasure chests (402, 409, 448, 557):** need `RunCountExchange`,
  `GetMaxExchange` and `RollDice`, and the chests have no drop source.
- **Helena and Murka's holy water (400, 410-412):** the reference events are
  incomplete.
- **Kaishan's redistribution (407):** needs the stat and skill reset (PROG-02).
- **Heppa crafting (130, 141, 146):** rewards are absent from 1453 ITEM.
- **Other unsynthesized quests:** Rothen (423/424), Laek (445), alternate
  suppression flags 602/604/606/608, and NPCs that are not populated (525,
  537-544, Prisia event 574-578).
- **Out of scope:** Hemes name services and client-only quests 529-535/701/702.
