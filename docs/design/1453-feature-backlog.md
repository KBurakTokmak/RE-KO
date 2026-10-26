# Remaining feature backlog

This is a selection menu, not authorization to port every private-server feature.
Completed work belongs in [README](../../README.md) and maintained subsystem notes;
completed task IDs are retired, never reused. Remaining IDs stay stable.
“Partial” names a bounded implementation with a specific gap; “Unverified” needs
original1453 evidence before deciding whether to implement it. P1/P2/P3 indicate
suggested priority, not scope commitments.

Source abbreviations: GS=`1453_New_2025/Source/GameServer`,
AI=`1453_New_2025/Source/AIServer`, DB=local supplied1453 exports,
Rust=`crates/reko-server/src`. These references are outside Git; consult
[reference data](../reference-data.md). The source packet dispatch is
`GS/User.cpp::HandlePacket`; source enums are not native1453 layout specifications.
Existing handlers do not establish complete retail behavior. Preserve the working
[world services](../re/world-services.md), [combat](../re/skills-combat.md),
[quests](../re/quests-progression.md), [social](../re/social.md),
[accounts](../re/accounts.md) and [skillbars](../re/skillbar.md).

## Quests and progression

| ID | Task | Status | Priority | Reference / verified gap | Depends on |
| --- | --- | --- | --- | --- | --- |
| QUEST-04 | Specialist and branching quest coverage audit | Partial | P1 | Generic kind1 helper accept/turn-in, counters, item exchange, selected kind12 rewards exist. `database_quest_script` does not synthesize every quest kind/action. Supplied1453 runtime Lua directory is absent; supplementary2086 scripts are not proof of1453 content. Inventory helpers/menus/API coverage before adding only verified missing actions. | ordinary quest infrastructure |
| QUEST-05 | Normal mastery and high-level skill unlock quests | Partial | P1 | Test characters have administrative unlocks; novice promotion exists. Rust has no general normal `PromoteUser` implementation comparable to GS `QuestHandler.cpp::PromoteUser`. DB flags alone do not prove quest requirements/payment/rewards. Trace each class's ordinary master/unlock chain. | QUEST-04 |
| PROG-01 | Death XP loss, delevel and resurrection XP refund | Missing | P1 | Rust `quest_rewards.rs::gain` handles positive rewards; `town.rs::respawn` restores life without a negative-XP ledger/refund path. GS `User.cpp::ExpChange`, `AttackHandler.cpp::Regene`, magic resurrection handling. Define original1453 death-type/zone exceptions before implementing. | CLASS-05 for resurrection refund |
| PROG-02 | Normal stat and skill reset services/fees | Missing | P1 | Source GS `NPCHandler.cpp::ClassChange` dispatches `AllPointChange`, `AllSkillPointChange`, fee quote. Rust allocates points but has no incoming0x34 reset dispatcher. First-job server0x34 reply is not a reset handler. | ordinary quest infrastructure |
| PROG-03 | XP/NP/coin modifiers from premium, equipment and server events | Partial | P3 | Base solo gains exist; GS `User.cpp::ExpChange/GoldGain`, king/event bonuses have additional paths. Rust base monster reward path applies damage share and level modifier, not all premium/global bonuses. Verify actual1453 items and choose which optional systems to retain. | ECON-02, WORLD-06 if selected |

## World, monsters and navigation

| ID | Task | Status | Priority | Reference / verified gap | Depends on |
| --- | --- | --- | --- | --- | --- |
| WORLD-01 | Terrain/shape collision and safe navigation | Missing | P1 | Rust `npc_ai.rs::move_toward` moves straight toward coordinates; there is no source SMD pathfinder/shape collision query. AI `Npc.cpp::PathFind`, `GetTargetPath`, `StepMove`, AI `Map.cpp`, shared SMD loader. Validate historical-map matches first; do not simply use2086 Moradon. | Historical map/geometry audit |
| WORLD-02 | Patrol paths/movement3 and remaining movement modes | Partial | P1 | Spawn path metadata is retained; runtime roaming is1/2 and stationary modes0/4. AI `Npc.cpp::RandomMove/GetNearPathPoint/IsInPathRange`, DB `K_NPCPOS` paths. Use existing source route data and WORLD-01 for obstacle-aware patrols. | WORLD-01 recommended |
| WORLD-03 | Monster skill casting, elemental/ranged special attacks and healing | Missing | P1 | Rust AI loads physical damage/range/delay and queues physical strikes, not prototype magic skill IDs through NPC magic execution. AI `NpcMagicProcess.cpp`, `MagicProcess.cpp`, `Npc.cpp::NpcHealing/SendAttackRequest`; DB monster magic fields. Start with observed mobs and verified native effect packets, then expand all supported types. | CLASS-06, WORLD-01 recommended |
| WORLD-04 | Family/boss assistance and NPC-versus-NPC targeting | Missing | P2 | AI `Npc.cpp::FindFriend`, `ChangeTarget`, `ChangeNTarget`, `FindEnemy`. Rust `npc_ai.rs` tracks players as targets and has no family-assistance state or NPC enemy registry. Preserve neutral-versus-aggressive rules. | WORLD-03 for healer assistance |
| WORLD-05 | Bind points, removal and saved respawn choice | Missing | P1 | GS `User.cpp::ObjectEvent` has `OBJECT_BIND/REMOVE_BIND`; Rust object dispatch is gate/anvil, and `town.rs::respawn` uses town start data. No persisted bindpoint selection. Needed for original Gate/Escape and resurrection-location rules. | WORLD-01 for safe destinations |
| WORLD-06 | Time/weather progression and associated gameplay bonuses | Missing | P3 | Rust has no0x13/0x14 update path or source timer/weather controller; client-local/default presentation does not establish server weather. GS `GameServerDlg.cpp::UpdateWeather`, `User.cpp::UpdateGameWeather`. Cosmetic clock and weather damage modifiers should be audited separately. | None |
| WORLD-07 | Dynamic event spawns, rooms, trap/door/lever/object controllers | Partial | P2 | Ordinary spawn/respawn works; room/family/event metadata is retained but not an event state machine. GS `User.cpp::ObjectEvent` has lever/flag/gate paths; AI map/room behavior controls special entities. Rust gate/anvil handling does not cover those branches. | WORLD-01; EVENT tasks as appropriate |
| WORLD-08 | Zone access, hostility, tariff and PvP rule matrix | Partial | P1 | Rust `world_entry.rs::zone_ability` only supplies Moradon rule packet; `travel.rs` validates supported destinations/nation/level but is not the full source policy. GS `CharacterMovementHandler.cpp::CanChangeZone/CanLevelQualify`, GS map zone flags, DB `ZONE_INFO`. Some configured zones being populated does not prove complete gameplay rules. | LAN-01, CLASS-01 for PvP |
| WORLD-09 | Complete radius/model range and line-of-sight equivalence | Unverified | P2 | Original skeleton radii and native interpolation are implemented; wall/height/LOS equivalence remains unverified. Different effects may require wall/height/LOS rules. Avoid replacing working attack reach with guessed uniform ranges. | WORLD-01 |

## Class, combat and item mechanics

| ID | Task | Status | Priority | Reference / verified gap | Depends on |
| --- | --- | --- | --- | --- | --- |
| CLASS-01 | Player-versus-player damage and hostility | Missing | P2 | Rust normal attacks address monster world IDs only; `magic.rs` explicitly rejects Armor Cancellation because player combat is unavailable. GS `AttackHandler.cpp`, `User.cpp::GetDamage`, `MagicInstance.cpp`, `Unit.cpp`. Requires living player lookup, zone/nation/arena rules, death/NP behavior and effect broadcasts. | LAN-01, WORLD-08 |
| CLASS-03 | Rogue player-specific effects and detection | Partial | P2 | Hide/stealth, self defense/cures/speed and attacks exist. Real Lupine/Cat's Eyes detection, Smoke Screen, Illusion, PvP secondary blind/armor cancellation/Critical Point and Wild Advent still require player targeting/warp/visibility. See Rust explicit guards and [class environment](../re/skills-combat.md); primary GS `MagicInstance.cpp` type4/8/9. Concentration accuracy semantics remain unverified, not an arbitrary hit bonus. | LAN-01, CLASS-01/02, WORLD-05 as relevant |
| CLASS-04 | Mage transport, shields/reflection and control effects | Partial | P1/P2 | Source-based transport, NPC lightning stun, reactive armor, MP drain/player control, friendly buffs and MP-bounded Mana Shield are implemented. Full player damage, exact retail freeze/slow probabilities, bind points, Blink collision/indoor geometry and player/monster-spell reflection remain incomplete; [Mage integration](../re/skills-combat.md#mage-integration-and-original1453-constraints). GS MagicInstance type3/4/8 and AIServer Npc::RecvAttackReq; documented reflection-source correction. | WORLD-01/05, LAN-01, CLASS-06 |
| CLASS-05 | Priest remaining curses, sleep, resurrection and player defenses | Partial | P1/P2 | Self heals/buffs/cures plus monster HP/AC curses work. Rust `priest.rs` only executes hostile type4 categories1/2. Confusion/Slow/Massive active NPC modifiers, type7 sleep, resurrection, remaining player curses and Elysian Web remain separate. GS `MagicInstance.cpp` type3/4/5/7/8; [priest findings](../re/skills-combat.md). | LAN-01, CLASS-06, PROG-01 as relevant |
| CLASS-06 | Common sleep/stun/root/slow/reflect and threat state execution | Partial | P1 | Existing NPC speed/AC/HP modifiers are subsets. Mage lightning stun/reactive armor are bounded implementations; generic sleep/root and other reflection paths remain. GS `MagicInstance.cpp` and AI `NpcMagicProcess.cpp`; effects need to suspend/resume appropriate AI actions and expire correctly, not only display an icon. | WORLD-03 interaction audit |
| CLASS-07 | Warrior player-dependent secondary effects | Partial | P2 | Basic physical/selected magic, self buffs and master reductions exist. Player durability destruction, complete stun/secondary utility and party effects are not established by successful primary damage. GS `MagicInstance.cpp`; Rust generic physical/secondary whitelist and [warrior findings](../re/skills-combat.md). | CLASS-01/02/06 |
| CLASS-08 | Exact attack hit/miss, cast/combo/launch timing audit | Partial | P2 | Costs, per-skill cooldowns, bounded paid projectile tickets and multi-arrow damage exist. Rust `magic.rs` explicitly leaves animation completion validation TODO; inspect native clips and Open-KO `MagicSkillMng.cpp` before imposing cancellation restrictions. Separate player-to-mob physical hit/evasion audit from already-present NPC physical hit rolls. | Native animation/packet capture |
| ITEM-02 | Equipment bonuses and on-hit effects | Partial | P1 | Rust `combat.rs` applies primary stats, direct HP/MP, resistances and accuracy/evasion from original extensions. Full elemental/leech/reflect/XP effects remain incomplete. GS `User.cpp::SetUserAbility/SetSlotItemValue/GetDamage`, `Unit.cpp`; audit column-by-column against original tables. | CLASS-01/06 for player effects |
| ITEM-03 | User bag stack merge/split semantics | Partial | P2 | Purchases/loot/storage can grow legal stacks, but Rust inventory direction3 uses `bag.swap`; arbitrary bag moves are not a native split/merge implementation. GS `ItemHandler.cpp::ItemMove` and client inventory send paths. Preserve source stack caps and atomic quantity checks. | None |
| ITEM-04 | Accessory compounding and remaining reverse upgrades | Partial | P3 | Normal upgrades and verified +7 Reverse Scroll conversion work; accessory compounding and remaining reverse branches need their own recipes/UI audit. GS `UpgradeHandler.cpp::ItemUpgradeAccessories/ItemUpgradeRebirth`; DB corresponding recipes. Confirm original1453 UI/items before enabling rebirth/private rows. | ITEM-02 recommended |
| ITEM-05 | Chaotic Generator and special item exchanges | Partial | P2/P3 | Native Chaotic Generator fragments and Hemes exchanges are implemented in `chaotic.rs`/`hemes.rs`. Audit remaining specialist exchanges individually; source `UpgradeHandler.cpp::BifrostPieceProcess/SpecialItemExchange`, documented in world services. | QUEST-04; EVENT-03 if access gated |
| ITEM-06 | Sealing/unsealing, binding and expiring/rental item metadata | Missing / Unverified | P3 | Item seal source logic exists (`SealHandler.cpp::ItemSealProcess/SealItem`), but Rust item records do not model the full source serial/flags/expiry lifecycle. Source rental packet handler is itself TODO and character sealing empty; see exclusions. Verify1453 interface and desired scope before adding data migrations. | Native applicability and schema audit |
| ITEM-08 | Permanent Valkyrie costume equipment and bonuses | Bounded / owner policy | P2 | The eight original Peddler variants equip in the two native auxiliary slots, persist and provide selected HP/AC/monster EXP/Noah bonuses. Owner explicitly requires permanent items. Timed conversion is deferred until requested, with per-instance metadata and migration policy required; see [services](../re/world-services.md). Other later costume families/slots are not implied. | ITEM-06 only for a future timed lifecycle |
| ITEM-07 | Durability threshold appearance and exact wear edge cases | Partial | P2 | Wear, broken equipment reduction and repair work. Full65/25-percent appearance transitions and killing-hit calculated-damage versus clamped-damage semantics were left open in [durability findings](../re/skills-combat.md). Source GS `User.cpp::ItemWoreOut`/appearance broadcast. | LAN-01 for other-player appearance |

## LAN, social and player economy

| ID | Task | Status | Priority | Reference / verified gap | Depends on |
| --- | --- | --- | --- | --- | --- |
| LAN-01 | Shared player registry, user region entry/exit and movement/appearance | Partial | P1 for LAN | Online snapshots, native IN/OUT, bounded requested-user details, movement/appearance and disconnect cleanup now exist. Accepted skill/attack/level visuals and target HP bars are synchronized. Rotation and general player-target combat remain separate. See [social systems](../re/social.md). | AUTH-01 for token binding |
| LAN-02 | Parties and party finder | Partial | P2 | Eight-member invite/consent, leave/kick/disband, promotion, HP/MP/level/class roster, party chat and ordinary group buffs/heals/cures implemented. Loot cursor survives membership/leader changes and clears on disband. Finder and complete retail status presentation remain. See [social systems](../re/social.md). | LAN-01, friendly skills |
| LAN-04 | Clans, grade/NP, symbols/capes and alliances | Partial | P2/P3 | Persistent nation-specific core clans, atomic creation fee, invite/consent, leave/kick/disband, chief/vice/officer roles, roster, login fields and clan chat implemented. Member NP determines grade; Grade3 Caitharos trial, atomic key/fee promotion, chief-owned trial loot and persisted native cape purchases implemented. NP donation, rankings, symbols, alliances, siege and isolated trial instances remain. See [social systems](../re/social.md). | LAN-01, AUTH-01 |
| LAN-05 | Private messaging, friends and social chat routing | Partial | P2 | Normal/shout and membership-restricted party/clan chat work. Private target selection, friends and alliance chat remain absent. | LAN-01/02/04 as appropriate |
| ECON-01 | Player-to-player trade | Implemented; native UI acceptance pending | P2 | Native0x30 consent/offers/both-confirm/cancel, post-exchange slot/stack/weight/coin simulation and one two-sided SQLite commit. Disconnect/teleport and external inventory changes cancel safely. Remote trade-board and pet/user-data payloads remain outside scope. See [social systems](../re/social.md). | LAN-01, AUTH-01 |
| ECON-02 | Player selling/buying merchants | Missing | P2/P3 | NPC shops work; no Rust0x68 player merchant dispatcher. GS `MerchantHandler.cpp` selling and buying flows. Buying merchant availability in native1453 must be verified before choosing that branch. Listings/reservation, atomic purchase and disconnect cleanup are required. | LAN-01, ITEM-03, AUTH-01 |
| ECON-03 | Letters, attachments and mailbox | Missing | P3 | No Rust0x6A shopping-mall/letter dispatch. GS `ShoppingMallHandler.cpp` routes letters; `LetterHandler.cpp` list/read/send/get/delete and database requests. Mail attachments require unique claim and transaction persistence. | AUTH-01; LAN-01 for live notifications |
| AUTH-01 | Password validation, login token binding and duplicate-session handling | Partial | P1 before LAN exposure | First-login registration and salted Argon2id verification are implemented, with locked per-server faction choice; see [accounts](../re/accounts.md). Rust `lobby.rs` still uses one-time IP association and rejects ambiguous same-IP pending logins; native token binding and duplicate live-session handling remain. Source GS `LoginHandler.cpp`/login server auth and `WIZ_KICKOUT` behavior. Implement SQLite-local authentication/session binding, not source SQL/ODBC infrastructure. | Native login-token evidence |
| CHAR-01 | Normal character deletion and appearance change | Missing | P2 | Source pre-world dispatcher includes `WIZ_DEL_CHAR`0x03 and `WIZ_CHANGE_HAIR`; Rust game dispatch creates/selects but lacks0x03 deletion flow. Manual prior SQL removal is not this feature. GS `CharacterSelectionHandler.cpp`, DB requests; ownership/confirmation and cascading records need tests. | AUTH-01 recommended |
| CHAR-02 | Name, nation, gender and job-change consumables | Missing / Unverified | P3 | GS `NPCHandler.cpp::HandleNameChange`, `CharacterHandler.cpp::NationChange/GenderChange/JobChange` have paths; no corresponding Rust dispatch/effects. Verify client support, item IDs, clan/party restrictions and need for each private service. | AUTH-01, ITEM-06 for metadata if needed |

## Events, PvP services and optional private features

| ID | Task | Status | Priority | Reference / verified gap | Depends on |
| --- | --- | --- | --- | --- | --- |
| EVENT-01 | Arena challenges, duels and clan challenges | Missing | P3 | GS `ArenaHandler.cpp::HandleChallenge*`; no Rust challenge dispatcher/session/arena controller. Requires consent, transfer, win/leave/timeout and anti-farming policies from original behavior. | CLASS-01, LAN-01; LAN-04 for clan duel |
| EVENT-02 | Nation wars, invasion, monuments and rankings | Missing | P3 | GS `GameServerDlg.cpp::BattleZone*`, movement ranking handlers and `User.cpp::HandlePlayerRankings`; imported war tile actions are deliberately gated without a controller. Need schedules, state transitions, score, kickout, nation gates and rewards. | CLASS-01, LAN-04, WORLD-07/08 |
| EVENT-03 | Forgotten Temple/Bifrost and timed dungeon waves | Missing / Unverified | P3 | GS `GameServerDlg.cpp::ForgettenTempleEventTimer/TempleEvent*`, `EventHandler.cpp::BifrostProcess/TempleProcess`; no Rust event scheduler. Historical versus private event types must be separated (Monster Stone/Chaos/Juraid-style additions are not automatically1453 requirements). Existing one-life spawn flags are not complete wave progression. | WORLD-03/07, LAN-02 for party events |
| EVENT-04 | Castle siege, ownership and tariffs | Missing | P3 | GS `EventHandler.cpp::CastleSiegeWarProcess`, `GameServerDlg.cpp::CastleSiegeWarZoneOpen/UpdateSiegeTax`, `User.cpp::SiegeWarFareNpc`. Rust has neither siege state nor clan ownership/tax policy. | EVENT-02, LAN-04, WORLD-07 |
| EVENT-05 | King election, treasury/tax and king events | Missing / Unverified | P3 | GS `KingSystem.cpp` implements election lists/votes, tax and special events; some impeachment methods are empty. No Rust source0x78 king dispatch/state. Confirm native opcode/layout rather than copying a versioned numeric value. | LAN-04, EVENT-02/04; AUTH-01 |
| OPTIONAL-01 | Cash shop/web mall, premiums and external delivery | Missing / Unverified | P3 | GS `ShoppingMallHandler.cpp` and database/web paths; Rust only has ordinary game items and currencies. User can choose to omit monetization entirely. Do not introduce external-account or SQL-server dependencies for the LAN project. | Explicit product decision |
| OPTIONAL-02 | GM operator tools, bans/mutes/teleports and announcements | Partial | P3 | Existing admin flag gates training commands, fixed basic R damage, monster protection, native GM collision and one-life monster summoning by ID/name with paged discovery; see [admin testing](../re/admin-testing.md); not full GS `User.cpp::OperatorCommand`, source chat/console commands and authority broadcasts. Keep debug/admin operations distinct from normal character mechanics. | AUTH-01, LAN-01 |
| OPTIONAL-03 | Helmet visibility, custom cape, Logos Shout and cosmetic services | Missing / Unverified | P3 | Native cape purchases are implemented. Remaining helmet visibility and Logos Shout require original-client applicability checks; source `HandleHelmet`/`LogosShout`. | LAN-01, ITEM-06 if consumable state |
| OPTIONAL-04 | Mining, soccer and other source-private systems | Unverified | P3 | GS `User.cpp::HandleMiningSystem` contains mining/soccer; `HandleMiningStart` explicitly calls itself a mock-up and omits area checks. Their existence in this source is not proof of original1453 gameplay. Audit client UI/maps first or deliberately omit. | Explicit feature selection |

## Source limitations

Do not create a “port everything” ticket from a method name alone:

- GS `RentalHandler.cpp::RentalSystem` reads an opcode and leaves both rental
  branches commented with `TODO`. Rental cannot be faithfully completed from it.
- GS `SealHandler.cpp::CharacterSealProcess` is empty. Character sealing needs
  a separate original-client/source investigation if selected.
- GS `ItemHandler.cpp::RunSelectExchange` is a placeholder sending an exchange
  request; blindly copying it is not a selected quest-reward implementation.
  Rust's ordinary selected reward path uses independently recovered native menu
  packing. Audit specialized exchanges separately.
- GS `KingSystem.cpp::ImpeachmentRequest/RequestElect/List/Elect` are empty even
  though other king functions are substantive.
- `WIZ_MAP_EVENT` is a no-op in the reference incoming switch. Real server tile
  events run from validated movement/`Map.cpp::CheckEvent`; a missing packet
  response is not by itself a missing gameplay feature.
- Source mining is explicitly a mock-up. Absent runtime1453 quest Lua and maps
  cannot be reconstructed by assuming every2086 script or map is interchangeable.
- Private extra inventory/cosmetic slots, newer classes, an83-level cap and
  multi-server ODBC/AI-server processes are not automatic requirements. RE-KO
  deliberately targets the original1453 client, four classes, cap80 and SQLite.
