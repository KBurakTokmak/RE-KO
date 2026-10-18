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

Native1453 parsers: player IN006A3D60, party006B47C0, clans006B8580, clan
creation006BB810, join006BC1C0, invite006BD3E0, member list005A3920 and loot
006B2C30. Open-KO `GameProcMain` corroborates packet semantics. Native player
IN has an extra byte before direction compared with Open-KO. Native party HP/MP
fields are16-bit, unlike the supplied private server's32-bit serialization.
General player-target combat, complete remote skill animations and PvP remain
separate from this presence and social implementation.

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
Party finder and full other-player/group skill effects are not implemented.

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

This is core training-clan membership, initially grade5. Promotion/NP donation,
rankings, symbols, capes, clan banks, alliances, wars and siege ownership remain
unimplemented. They do not follow merely from receiving a clan ID. Specialist
clerk/ranking services remain separate from the Inn creation and clan member UI.
