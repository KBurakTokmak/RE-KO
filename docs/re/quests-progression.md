# Quests, rewards and progression

Ordinary kind1 quests use imported1453 QUEST_HELPER, ITEM_EXCHANGE and
QUEST_MONSTER requirements/rewards. States are0/unset,1/accepted,2/completed,
3/ready and4/abandoned. NPC menus and reward choices are scoped to offered
actions. Rewards, costs, items and progression commit atomically; replaying a
completed menu cannot pay again. Specialist quests remain partial.

## Quest journal commands

In ordinary chat, type `quests` (or `quests 2` for the next page) to privately
list this character's accepted/ready quests, eight per page. `quest <ID>` shows
the original Quest_guide_us.tbl title/accepted description and independent
monster counters. Quests without a matching guide title retain their numeric
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

High-level skill gates use persisted completed quest flags. Admin fixture
unlock scripts are separate from normal quest completion; mastery and specialist
unlock chains are not all implemented. Consult the feature backlog before
claiming an entire NPC or class is complete.
