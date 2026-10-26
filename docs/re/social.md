# Parties, clans and online players

## Online state and native packets

`social.rs` owns online-player snapshots and per-player notification queues.
Player IN/OUT uses native0x07; validated movement, equipment/class/clan changes
and region transitions update nearby clients. User-detail0x16 requests are bounded,
deduplicated and restricted to nearby online IDs. Zone-loading clients receive
world entries after loading completes. Disconnects clear presence and pending
invitations; selecting a character already active on another connection is refused.
Notifications flush before a new transaction and after processing/ticks, preserving
ordering between external party rewards and the player's own shop/loot replies.
Queued updates also wake the recipient's socket task immediately; remote effects
and vitals do not wait for the250ms character tick.

Accepted movement relays validated coordinates, signed speed and start/stop
flags, including a stop at unchanged coordinates. Snapshot synchronization no
longer invents a constant45-speed move for every changed position. Teleports
invalidate both sides' visibility caches and rebuild native OUT/IN and region
records even within the same region, without exposing hidden GMs.
The connection's online-player guard stays registered during zone loading.
Cross-zone travel explicitly removes the travelling character from its party
before the loading handshake, including pending invites/summons. Returning does
not rejoin. A departing leader promotes the next member when at least two members
can remain; a two-member party disbands. Same-zone warps/summons retain membership.
This follows primary1453 CharacterMovementHandler.cpp::ZoneChange's leader
promotion and PartyRemove path; it is separate from socket-disconnect cleanup.

`visuals.rs` collects server-produced attack08, level1B and validated magic31
cast/fly/effect/failure replies during actions/ticks. Only visible nearby
observers receive them; source IDs must match the actor, skills must exist in the
catalog and hidden targets stay hidden. Incoming requests, private resource/items
and client-claimed damage are never broadcast. Friendly recipients keep their
direct effect without a second observer copy. Primary1453 User.cpp::LevelChange
and MagicInstance.cpp region sends, corroborated by Open-KO GameProcMain's remote
level/attack/magic receivers, define these visual packets.

Target22 replies cover visible players, monsters and ordinary NPCs using native
ID:u16, immediate:u8, maximum/currentHP:u32 and delta:i16. Player target watchers
receive HP changes independently of party membership; noncombat NPCs use imported
HP. Monster snapshots share actual maximum/current HP and set immediate1 for
prompt bars. Target22's delta is personal: Open-KO/native client logs any negative
delta as damage dealt by the local player, with no caster field. Only the credited
character receives its own direct/DoT/curse HP loss; observers get delta0. Combined
DoT events retain separate owner portions. This changes presentation, not damage,
resource checks, XP or loot attribution. No extra periodic monster polling is added.
Source1453 User::SendTargetHP and Open-KO GameProcMain::MsgRecv_TargetHP corroborate
widths and echo behavior.

Native0D/1 builds the character snapshot; matching0D/2 confirms that the client's
world load completed. Stage2 requires a prior snapshot/entry and refreshes player
regions without reissuing the game-start acknowledgement, inventory, quests or
buff restoration. The previous stage1-only guard rejected this real follow-up.
Primary1453 CharacterSelectionHandler.cpp::GameStart distinguishes both stages.
On completion, refresh the viewer and nearby observers with native0x15 IDs;
region membership changes and town/respawn refreshes also send this list.
Include the viewer's own ID, as primary RegionUserInOutForMe does: the native
parser skips self and uses a self-only list to clear stale players, while count0
returns early. Native0x16 then retrieves missing records with no per-record
padding, as original0069C1D0/006A3D60 requires. Supplied custom GetRegionUserIn
prepends a zero byte that this original parser does not consume; do not copy it.
The three-client socket regression covers same/opposite nations in Moradon,
distant-player exclusion, stage ordering/replay and exact record consumption.

Native1453 parsers: player IN006A3D60, party006B47C0, clans006B8580, clan
creation006BB810, join006BC1C0, invite006BD3E0, member list005A3920 and loot
006B2C30. Open-KO `GameProcMain` corroborates packet semantics. Native player
IN has an extra byte before direction compared with Open-KO. Native party HP/MP
fields are16-bit, unlike the supplied private server's32-bit serialization.
GM visibility is filtered on the server for each viewer. A normal player receives
neither region IDs, IN/movement/equipment presence nor requested16 records for a
hidden GM; other GMs can see that GM. All chat command `gmvisible on` permits
ordinary viewers, `gmvisible off` restores hiding and `gmvisible status` reports
the state privately. Every command rechecks SQLite authorization. The override
is session-local, defaults to hidden and clears on disconnect or role revocation.
Membership changes emit native OUT/IN and complete viewer-specific region lists.
Remote07/16 records that are permitted use visible-player authority1.
Local0E retains GM0 for the owner's native collision bypass;
the server's character_privileges role remains unchanged. Original00487646..
00487666 skips remote GM0 rendering when normal abnormal-state1 is below5;
0048876D/004895E8 also exclude GM0 from selection. A read-only multi-client
inspection found the other player already in each native manager, with correct
IDs/nearby coordinates but authority0. Primary User.cpp::ResetGMVisibility
corroborates retail GM invisibility, while current Open-KO has commented-out
render exclusions and must not override original1453 behavior here. Keeping GMs
visible to authorized viewers is an intentional local display policy; it neither
revokes their server permissions nor grants privileges to other clients.
General player-target combat and PvP remain
separate from this presence and social implementation.

## Player trading

Native0x30 supports request1, consent2, add3/other-add4, confirm5/other-confirm6,
complete7 and cancel8. Use the player's native Trade control while nearby.
Each player can be in only one pending or active exchange: both participants
are checked and reserved under the same Hub lock, so simultaneous or crossed
requests cannot create overlapping trades. There are twelve item offer slots
plus Noah. Both players must be alive, loaded,
within10m and visible to one another. Nations must match except in Moradon or
Forgotten Temple, as with existing zone social rules. Requests expire after60s;
active windows after10minutes. These are bounded local session policies.

Offers stay in the owner's saved bag/wallet until completion. Add validates
ownership, quantities, original item/extension and untradeable race20; IDs
at/above900000000 are excluded except the explicit Noah offer. Both confirmations
are required, and either confirmation freezes both offers. Final validation reads
both current inventories/wallets, removes both outgoing offers, then simulates
incoming stack merges, free slots, durability, weight and the2100000000 coin cap.
Native countable1 stacks cap9999; small-countable3 cap500. Nonstackable charged
packs transfer whole without merging; the native unit-count1 request moves all
remaining charges with that icon (Open-KO UIPerTradeDlg::ReceiveIconDrop). All four inventory/wallet writes commit
in one SQLite transaction; validation/write failure rolls everything back and
sends native failure plus a private reason to both players. Success sends incoming
item placements and final Noah; the trade UI owns its icon changes, with no duplicate
inventory snapshot. Session caches refresh from committed data before later actions.

Cancel, decline, disconnect, death, excessive distance and teleport discard offers.
Periodic timestamp41 and other read-only traffic do not cancel exchange. Cancel
captures record both participants, the reason and triggering gameplay opcode.
Other inventory/service/combat actions end exchange before dispatch; autoloot pauses
during exchange. External inventory/Noah notifications end it before applying their
update, preserving party loot rather than withholding rewards. No schema migration,
startup grant or persistent half-trade state is needed.

Evidence: supplied1453 GameServer/TradeHandler.cpp, Open-KO GameProcMain and
SubProcPerTrade, original1453 receiver006B5900 and item insertion00634170. Original
agree/add results are bytes; other-add includes a trailing u32 user-data length0.
Completion is success:u8,coins:u32,count:u16, then destination:u8,item:u32,
quantity:u16,durability:u16,user-data-length:u32 per incoming placement. The native
receiver consumes this tail unlike Open-KO1298; nonzero pet/user data is outside
the current item model. Ordinary nearby trade requests accept source four-byte
form or the native five-byte form with near-mode1; remote trade-board mode is
not implemented.

## Parties

Use the client's party controls to invite by player name. Native0x2F supports
create/invite, explicit accept/reject, remove/leave, disband and leader promotion.
Maximum membership is eight. The leader may invite/kick/promote; a member may
leave. Leader departure or reducing a party to one member disbands it, following
supplied1453 `PartyHandler.cpp`. Parties and pending offers are runtime state.
Offers expire after60seconds as a bounded local consent policy.

Invitation requires the same zone. Opposite nations may party in Moradon or
Forgotten Temple; elsewhere nations must match. Unless members share a clan,
the invited level must be within two-thirds to1.5times the inviter's level or
within eight levels. HP/MP, level and class changes update the party roster.
Party chat routes only to members, including across zones.

Monster death aggregates each party's contributors once. The party's base XP/NP
is its damage share, rounded up. Supplied1453 `Npc::GetPartyRewardModifier` uses
monster level minus average party level: at least8/5/2 gives2.0/1.5/1.2times;
otherwise1.0. Unlike solo rewards, the party branch has no low-level-monster
penalty. XP then receives30percent per additional member; NP20percent. Each
member's share is proportional to their level divided by total party levels,
rounded up. Only living, loaded members within50m of the corpse receive rewards.
Out-of-range members remain in the source denominator and do not receive a share.
Source: `User.cpp::RecvUserExp`, `Npc.cpp`, Open-KO AI party XP handling.

The existing largest-contributor kill owner shares ordinary quest kill credit
with eligible nearby party members; each member's own active objectives remain
independent. Solo ownership and drop generation/rates are unchanged. Bundles
record their eligible party names at death, so joining afterward does not grant
access to that corpse. This is an explicit local ownership restriction.

Noah pickup splits coins by party levels using source integer truncation; only
living members inside the existing loot range receive their portions. All wallet
updates commit atomically before the corpse entry is consumed. Items rotate
through eligible members, skipping failed capacity/weight/life/range checks;
failed routing preserves the item. Native party loot results5/3 include a bundle
ID, unlike solo result1; result4 releases the looter when someone else receives
an item. Recipient inventory/coins refresh from SQLite before their next action.
Source: supplied1453 `User::ItemGet/GetItemRoutingUser`, original client006B2C30.
Loot rotation preserves the next recipient across leader promotion and member
removal, and clears on disband/new-party creation. It cannot inherit a previous
party's cursor. Regression coverage includes rotation, capacity/range/life
exclusions, atomic coin rollback, XP level weights and nearby quest credit.

Ordinary friendly/party buffs, heals, periodic restores and cures use the original
skill morals and table radii; see [skills and combat](skills-combat.md).
Caster payment and recipient saves are atomic, with party vitals refreshed after
commit. Party finder and resurrection remain
unimplemented; core membership/rewards do not imply every retail social feature.

## Clans

Create a clan through the Inn Host's **Create clan** option in your own home
castle. Imported locations include Galid at368,1688 in Luferson and Lined at
1688,370 in El Morad. Requirements follow supplied1453
`KnightsManager::CreateKnights` and `GameDefine.h`: level20,500000Noah, no existing
membership, unique name up to20bytes. Creation is restricted to the character's
own nation home zone. The fee and clan/chief records commit in one transaction.

SQLite `clans` and `clan_members` persist membership independently of online
sessions. A database trigger and invitation checks enforce matching nations.
A character belongs to at most one clan; a partial unique index permits one
chief. Clan IDs follow source nation ranges: Karus1..14999, El Morad15001..30000.
Maximum membership is36. Invitations require a nearby living same-nation player,
explicit consent and a chief/vice-chief whose current membership is revalidated.
Invite offers expire after60seconds; forged, stale and replayed consents fail.

Native0x3C supports creation, invitations/acceptance, leave, kick (including offline
members), disband, admit/reject, punishment, chief handover, vice-chief/officer
roles and member roster. Vice-chief appointments are limited to three. Changes
are loaded into MYINFO on login and into online appearance/chat routing. Clan chat
is restricted to matching membership across zones. Ordinary members cannot change
roles or disband; leader withdrawal disbands the clan as in the source.

Clan grade follows the sum of members' current national points:77000/144000/
360000/720000 for grades4/3/2/1. These are supplied1453 GameServerDlg defaults;
GetKnightsGrade does not divide the sum by36. Open-KO1298 EbenezerApp instead
divides by24 and uses2000/5000/10000/20000; those older thresholds are not
applied here. Its KnightsManager corroborates promoted-clan/cape serialization.

## Knight promotion and capes

At the nation's knight clerk, choose **I want to earn knight privileges**.
The original4150/4250 clerk menu remains in use; its mission option enters a
server-owned dialog using original1453 talks4165/4168/4172 and4265/4268/4272.
The supplementary2086 intermediate talks6382/6421 are absent from1453 and must
not be sent. They previously caused a native client crash. Other clerk services
retain their existing handling; specialist ranking rewards remain incomplete.

Only the current chief of a training clan of Grade3 or better may start the
trial. Acceptance saves the chief and living online clan members on mainland
zones below30 into zone54 at150,150, then sends the native loading handshake.
All positions commit together before transfer notifications. Solo chiefs may
enter. Reentry repeats eligibility checks. These are shared source populations,
not isolated instances; no extra horse or immediate respawn is manufactured.

Caitharos2690 uses the imported ordinary loot rule for Key of Honor910045000.
For a clan's kill in zone54, its bundle belongs to the online chief rather than
the party's item rotation: original talk4168 explicitly requires personal pickup.
The existing bag capacity, range and replay checks apply. The nearby Judge13010
offers return to the nation's imported home-town start. Judge's supplementary
2086 return to newer Moradon21/817,451 is deliberately not copied.

Back at the knight clerk, choose **I brought what you asked for** and confirm.
The current chief, grade and training status are revalidated from SQLite.
One key,10000000Noah and rank2/default cape0 commit in one transaction.
Failure rolls everything back; a repeated claim cannot consume more resources.
Grade remains based on national points; promotion changes clan type, not NP.
SQLite clan_progress persists rank/cape and cascades on clan deletion.

Promoted chiefs can use type25 cape merchants such as Habice. Prices and grade
requirements come from original Cloak.tbl; cape99 is unavailable. Native70
success/error and3C24 clan updates follow supplied1453 NPCHandler and
Knights::SendUpdate, corroborated by Open-KO KnightsManager. Purchases require
a nearby cape merchant, current leadership, sufficient coins and grade; the
wallet and cape commit together. Selecting the current cape costs nothing.
MYINFO and player presence include the saved cape after reconnecting.
Native006A0188 stores the pre-name byte as clan type;006A02D4/006A02FA
read grade/ranking after the name and006A03C8 passes them to KnightsInfoSet.
The previous MYINFO grade/type labels were reversed. They now match original
native consumption and supplied1453 User::SendMyInfo; player IN has no type byte.

NP donation, rankings, symbols, clan banks, alliances, wars and siege ownership
remain unimplemented. Do not infer these from basic knight promotion.
