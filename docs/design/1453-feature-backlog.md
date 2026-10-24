# Supplied1453 feature inventory and selectable backlog

Audit date: 2026-10-16 (Europe/Istanbul). This is a development menu, not an
instruction to implement every private-server feature. Task IDs stay stable so
we can choose a small batch later. Reference paths are local inputs outside this repository.

## Scope and evidence

The primary inventory is the actual incoming-packet dispatch in
``1453_New_2025/Source/GameServer/User.cpp``
(`CUser::HandlePacket`), its subsystem handlers, the AI implementation and their
loaders. Compare it with
[`lib.rs`](../../crates/reko-server/src/lib.rs),
[`lobby.rs`](../../crates/reko-server/src/lobby.rs), and the Rust modules listed
below. The dispatch comparison establishes absent entry points; an existing
entry point does **not** establish all subcommands/effects are complete.

This is a broad subsystem/packet-path audit, not a proof of every branch in the
roughly forty reference GameServer translation units. Per-skill/per-quest/per-map
acceptance still needs targeted native1453 tests. No completion percentage is
claimed. “Implemented” means the named bounded path exists, not full equivalence
with every behavior of the reference. Older notes sometimes say AI, XP, gates
or quests are wholly absent; current code and the latest HANDOVER supersede them.

Reference roots used below:

- **GS**: `../../../1453_New_2025/Source/GameServer/`.
- **AI**: `../../../1453_New_2025/Source/AIServer/`.
- **shared**: `../../../1453_New_2025/Source/shared/`.
- **DB**: `../../../PV_1453/inspection/` (exported supplied database rules).
- **Rust**: `../../crates/reko-server/src/`.
- **Open-KO**: `../../../KnightOnline/src/Server/` and
  `../../../KnightOnline/src/Client/WarFare/`.

Open-KO corroboration consulted: `Ebenezer/User.cpp`,
`Ebenezer/KnightsManager.cpp`, `AIServer/NpcThread.cpp` and client `PacketDef.h`.
Required navigation/compatibility notes are
[reference data](../reference-data.md). The supplied source contains
later/private additions despite its version constant. Check the original client
before accepting its balance, packet widths, item IDs or event availability.

Statuses:

- **Implemented**: the bounded path exists and has local validation; preserve it.
- **Partial**: a working subset exists; the gap column is the task.
- **Missing**: a source path exists but its Rust entry point/state/effect is absent.
- **Unverified**: source or native applicability is insufficient for an implementation decision.
- **In progress**: the current quest/UI repair is investigating this user report;
  do not infer this document's earlier observations are the final deployed state.

Priority is suggested, not authorization: **P0** current regression; **P1** core
leveling/class/world; **P2** LAN/social/economy; **P3** optional endgame/custom work.

## Preserve these working foundations

| ID | Bounded feature | Status | Evidence / practical limit |
| --- | --- | --- | --- |
| BASE-01 | Account nation, character creation/selection, starter items and persistence | Implemented | Rust `characters.rs`, `starter.rs`, `world_entry.rs`; GS `CharacterSelectionHandler.cpp`, `DBAgent.cpp`. Ordinary starting gold stays unchanged. Login security is separate below. |
| BASE-02 | Solo physical, dagger, bow, elemental and selected utility skills | Partial | Rust `magic.rs`, `archery.rs`, `mage.rs`, `priest.rs`, `utility.rs`; source GS `MagicInstance.cpp`, `MagicProcess.cpp`, `Unit.cpp`. Individual unsupported effects explicitly refuse; see CLASS tasks. |
| BASE-03 | HP/MP potions, standing/sitting regeneration | Implemented | Rust `resources.rs`, `skills.rs`, `utility.rs`; original-client skill/item data. User confirms potions work. Do not rebalance in unrelated work. |
| BASE-04 | Imported NPC/monster population, region streaming, ordinary respawns | Implemented | Rust `population.rs`, `monsters.rs`, `npc_ai.rs`; AI `ServerDlg.cpp`/`Npc.cpp`, DB `K_NPCPOS`, `K_NPC`, `K_MONSTER`. This covers 6568 imported instances, not event scheduling/collision. |
| BASE-05 | Monster physical aggression/retaliation, pursuit/return and idle roaming1/2 | Implemented | Rust `npc_ai.rs`; source AI `Npc.cpp::FindEnemy`, `ChangeTarget`, `NpcTracing`, `RandomMove`. User-requested fresh-hit pursuit renewal documented in [roaming findings](../re/world-services.md). Other AI modes below. |
| BASE-06 | Solo corpse boxes, original drop tables, coin/item pickup | Implemented | Rust `loot.rs`, `loot_generation.rs`; AI `Npc.cpp::GiveNpcHaveItem`, GS `User.cpp::BundleOpenReq/ItemGet`, DB drop/generator tables. Party ownership/distribution and unverified expiry are separate. Freeze existing solo behavior. |
| BASE-07 | NPC buy/sell/repair and account warehouse | Implemented | Rust `shops.rs`, `storage.rs`; GS `NPCHandler.cpp::ItemTrade/ItemRepair`, `ItemHandler.cpp::WarehouseProcess`. Warehouse items/coins are transactional and account scoped. |
| BASE-08 | Equipment footprints, pants/dual ring slots, swaps, item deletion | Implemented | Rust `inventory.rs`, `combat.rs`; GS `ItemHandler.cpp`. General equip requirements and stack manipulation remain separate. |
| BASE-09 | Normal anvil upgrades/preview, fees/reagents, original rates | Implemented | Rust `upgrading.rs`; GS `UpgradeHandler.cpp::ItemUpgrade`, DB upgrade tables. Accessory/rebirth/sealing are separate branches. |
| BASE-10 | Town menu and /town, clickable gates, Eslant tile teleports | Implemented | Rust `town.rs`, `travel.rs`; GS `Map.cpp::CheckEvent`, movement handlers; DB `EVENT` plus verified historical SMD tiles. This does not imply every event-zone travel controller exists. |
| BASE-11 | Positive solo monster XP/NP, level-ups,80-cap and point allocation | Implemented | Rust `quest_rewards.rs`, `progress.rs`, `stat_points.rs`; GS `User.cpp::RecvUserExp/ExpChange`, DB `LEVEL_UP`/`K_MONSTER`. All80 thresholds compared with original client. Quest notifications/tracking are covered by QUEST-01..03. |
| BASE-12 | First level10 novice job change | Implemented | Scoped Kaishan flow in Rust `npc_scripts.rs`; GS `QuestHandler.cpp::PromoteUserNovice`, DB quest406/exchange460. Preserve normal payment/requirements; it is not mastery. |

## Current quest and progression work

| ID | Task | Status | Priority | Reference / verified gap | Depends on |
| --- | --- | --- | --- | --- | --- |
| QUEST-01 | Immediate quest Noah reward/payment display | Implemented; retain regression coverage | P0 | Corrected weather0x14 to native gold0x4A/direction/delta/balance in quest, gate and anvil transactions. Source GoldGain/GoldLose and native006BA790 agree; regression verifies immediate reward/payment packets. See [findings](../re/quests-progression.md). | None |
| QUEST-02 | Quest XP grant and immediate XP/level display | Verified for reported quest | P0 | Patric Bulcan quest321 grants500XP, matching original client/imported exchange. Live trace and persisted XP confirm the grant; user clarified only the missing chat line was observed. Existing native1A XP notification retained; no reward changes or replay grants. See [findings](../re/quests-progression.md). | None |
| QUEST-03 | Native kill/item objective progress and quest tracking | Ordinary path implemented | P0 | Restored active quest:u16 in640901 snapshot; exact capped kill updates retained. Native6404/640C now validate owned ordinary objectives remotely, updating readiness without rewards/dialogs or response loops. Existing bag updates drive native collection counts. Follow-up supports40 concurrent accepted quests with independent kill counters; see [concurrency](../re/quests-progression.md). Specialist coverage remains QUEST-04. See [findings](../re/quests-progression.md). | None |
| QUEST-04 | Specialist and branching quest coverage audit | Partial | P1 | Generic kind1 helper accept/turn-in, counters, item exchange, selected kind12 rewards exist. `database_quest_script` does not synthesize every quest kind/action. Supplied1453 runtime Lua directory is absent; supplementary2086 scripts are not proof of1453 content. Inventory helpers/menus/API coverage before adding only verified missing actions. | QUEST-01..03 |
| QUEST-05 | Normal mastery and high-level skill unlock quests | Partial | P1 | Test characters have administrative unlocks; novice promotion exists. Rust has no general normal `PromoteUser` implementation comparable to GS `QuestHandler.cpp::PromoteUser`. DB flags alone do not prove quest requirements/payment/rewards. Trace each class's ordinary master/unlock chain. | QUEST-04 |
| PROG-01 | Death XP loss, delevel and resurrection XP refund | Missing | P1 | Rust `quest_rewards.rs::gain` handles positive rewards; `town.rs::respawn` restores life without a negative-XP ledger/refund path. GS `User.cpp::ExpChange`, `AttackHandler.cpp::Regene`, magic resurrection handling. Define original1453 death-type/zone exceptions before implementing. | CLASS-05 for resurrection refund |
| PROG-02 | Normal stat and skill reset services/fees | Missing | P1 | Source GS `NPCHandler.cpp::ClassChange` dispatches `AllPointChange`, `AllSkillPointChange`, fee quote. Rust allocates points but has no incoming0x34 reset dispatcher. First-job server0x34 reply is not a reset handler. | QUEST-01 |
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
| WORLD-09 | Complete radius/model range and line-of-sight equivalence | Unverified | P2 | Rust `monsters.rs` still notes collision-radius recovery; source model bulk/range and existing margins are only part of exact native reach. Different effects may require wall/height/LOS rules. Avoid replacing working attack reach with guessed uniform ranges. | WORLD-01 |

## Class, combat and item mechanics

| ID | Task | Status | Priority | Reference / verified gap | Depends on |
| --- | --- | --- | --- | --- | --- |
| CLASS-01 | Player-versus-player damage and hostility | Missing | P2 | Rust normal attacks address monster world IDs only; `magic.rs` explicitly rejects Armor Cancellation because player combat is unavailable. GS `AttackHandler.cpp`, `User.cpp::GetDamage`, `MagicInstance.cpp`, `Unit.cpp`. Requires living player lookup, zone/nation/arena rules, death/NP behavior and effect broadcasts. | LAN-01, WORLD-08 |
| CLASS-02 | Other-player and party heals/buffs/cures/speed | Implemented bounded | P2 | `friendly.rs` applies original morals2/3/4/6 and table ranges/radii to ordinary Type3 recovery, Type4 buffs/speed and Type5 cures. One caster payment and all recipient saves commit atomically; received buffs cancel without foreign-class learning. Counter Curse protects existing player Type4 control. Resurrection and complete remote animations remain separate. See [skills](../re/skills-combat.md). | LAN-01; LAN-02 for party |
| CLASS-03 | Rogue player-specific effects and detection | Partial | P2 | Hide/stealth, self defense/cures/speed and attacks exist. Real Lupine/Cat's Eyes detection, party Wolf, Smoke Screen, Illusion, PvP secondary blind/armor cancellation/Critical Point and Wild Advent still require player targeting/warp/visibility. See Rust explicit guards and [class environment](../re/skills-combat.md); primary GS `MagicInstance.cpp` type4/8/9. Concentration accuracy semantics remain unverified, not an arbitrary hit bonus. | LAN-01, CLASS-01/02, WORLD-05 as relevant |
| CLASS-04 | Mage transport, shields/reflection and control effects | Partial | P1/P2 | Source-based transport, NPC lightning stun, reactive armor, MP drain/player control, friendly buffs and MP-bounded Mana Shield are implemented. Full player damage, exact retail freeze/slow probabilities, bind points, Blink collision/indoor geometry and player/monster-spell reflection remain incomplete; [Mage integration](../re/skills-combat.md#mage-integration-and-original1453-constraints). GS MagicInstance type3/4/8 and AIServer Npc::RecvAttackReq; documented reflection-source correction. | WORLD-01/05, LAN-01, CLASS-06 |
| CLASS-05 | Priest remaining curses, sleep, resurrection and player defenses | Partial | P1/P2 | Self heals/buffs/cures plus monster HP/AC curses work. Rust `priest.rs` only executes hostile type4 categories1/2. Confusion/Slow/Massive active NPC modifiers, type7 sleep, other-player heals, resurrection, player MP effects, counter-curse/refraction and Elysian Web remain separate. GS `MagicInstance.cpp` type3/4/5/7/8; [priest findings](../re/skills-combat.md). | LAN-01, CLASS-06, PROG-01 as relevant |
| CLASS-06 | Common sleep/stun/root/slow/reflect and threat state execution | Partial | P1 | Existing NPC speed/AC/HP modifiers are subsets. Rust `npc_ai.rs` has no generic sleep/stun/root control state or reflected damage transaction. GS `MagicInstance.cpp` and AI `NpcMagicProcess.cpp`; effects need to suspend/resume appropriate AI actions and expire correctly, not only display an icon. | WORLD-03 interaction audit |
| CLASS-07 | Warrior player-dependent secondary effects | Partial | P2 | Basic physical/selected magic, self buffs and master reductions exist. Player durability destruction, complete stun/secondary utility and party effects are not established by successful primary damage. GS `MagicInstance.cpp`; Rust generic physical/secondary whitelist and [warrior findings](../re/skills-combat.md). | CLASS-01/02/06 |
| CLASS-08 | Exact attack hit/miss, cast/combo/launch timing audit | Partial | P2 | Costs, per-skill cooldowns, bounded paid projectile tickets and multi-arrow damage exist. Rust `magic.rs` explicitly leaves animation completion validation TODO; inspect native clips and Open-KO `MagicSkillMng.cpp` before imposing cancellation restrictions. Separate player-to-mob physical hit/evasion audit from already-present NPC physical hit rolls. | Native animation/packet capture |
| ITEM-01 | Full class/race/level/stat equip requirements | Implemented bounded | P1 | Incoming gear and displaced bag gear validate native race/class/subclass, base allocated stats, rank/title and imported server ITEM level bounds. Rank/title currently model zero. Slot/footprint handling remains intact; see [world services](../re/world-services.md). | Rank/title progression if selected |
| ITEM-02 | Equipment bonuses and on-hit effects | Partial | P1 | Rust `combat.rs` applies primary stats, direct HP/MP, resistances and accuracy/evasion from original extensions. Full elemental/leech/reflect/XP effects remain incomplete. GS `User.cpp::SetUserAbility/SetSlotItemValue/GetDamage`, `Unit.cpp`; audit column-by-column against original tables. | CLASS-01/06 for player effects |
| ITEM-03 | User bag stack merge/split semantics | Partial | P2 | Purchases/loot/storage can grow legal stacks, but Rust inventory direction3 uses `bag.swap`; arbitrary bag moves are not a native split/merge implementation. GS `ItemHandler.cpp::ItemMove` and client inventory send paths. Preserve source stack caps and atomic quantity checks. | None |
| ITEM-04 | Accessory compounding and rebirth upgrades | Missing | P3 | Normal anvil path exists. Rust `upgrading.rs` explicitly separates unsupported accessory/rebirth classes. GS `UpgradeHandler.cpp::ItemUpgradeAccessories/ItemUpgradeRebirth`; DB corresponding recipes. Confirm original1453 UI/items before enabling rebirth/private rows. | ITEM-02 recommended |
| ITEM-05 | Chaotic Generator and special item exchanges | Partial | P2/P3 | Generic validated NPC item exchanges exist; this is not GS `UpgradeHandler.cpp::BifrostPieceProcess/SpecialItemExchange`, which has distinct protocol, probabilities and effects. Implement per native service, keeping normal anvil and quest exchanges separate. | QUEST-04; EVENT-03 if access gated |
| ITEM-06 | Sealing/unsealing, binding and expiring/rental item metadata | Missing / Unverified | P3 | Item seal source logic exists (`SealHandler.cpp::ItemSealProcess/SealItem`), but Rust item records do not model the full source serial/flags/expiry lifecycle. Source rental packet handler is itself TODO and character sealing empty; see exclusions. Verify1453 interface and desired scope before adding data migrations. | Native applicability and schema audit |
| ITEM-08 | Permanent Valkyrie costume equipment and bonuses | Bounded / owner policy | P2 | The eight original Peddler variants equip in the two native auxiliary slots, persist and provide selected HP/AC/monster EXP/Noah bonuses. Owner explicitly requires permanent items. Timed conversion is deferred until requested, with per-instance metadata and migration policy required; see [services](../re/world-services.md). Other later costume families/slots are not implied. | ITEM-06 only for a future timed lifecycle |
| ITEM-07 | Durability threshold appearance and exact wear edge cases | Partial | P2 | Wear, broken equipment reduction and repair work. Full65/25-percent appearance transitions and killing-hit calculated-damage versus clamped-damage semantics were left open in [durability findings](../re/skills-combat.md). Source GS `User.cpp::ItemWoreOut`/appearance broadcast. | LAN-01 for other-player appearance |

## LAN, social and player economy

| ID | Task | Status | Priority | Reference / verified gap | Depends on |
| --- | --- | --- | --- | --- | --- |
| LAN-01 | Shared player registry, user region entry/exit and movement/appearance | Partial | P1 for LAN | Online snapshots, native IN/OUT, bounded requested-user details, movement/appearance and disconnect cleanup now exist. Full remote skill/attack animations, rotation and player-target combat remain separate. See [social systems](../re/social.md). | AUTH-01 for token binding |
| LAN-02 | Parties and party finder | Partial | P2 | Eight-member invite/consent, leave/kick/disband, promotion, HP/MP/level/class roster, party chat and ordinary group buffs/heals/cures implemented. Loot cursor survives membership/leader changes and clears on disband. Finder and complete retail status presentation remain. See [social systems](../re/social.md). | LAN-01, CLASS-02 |
| LAN-03 | Party XP/quest credit and loot distribution | Implemented bounded | P2 | Group damage is aggregated once; source average-level modifiers, XP/NP bonuses and level shares, nearby quest kills, atomic Noah shares and item rotation work. Solo generation/rates unchanged. Premium/event modifiers remain PROG-03. See [social systems](../re/social.md). | LAN-02 |
| LAN-04 | Clans, grade/NP, symbols/capes and alliances | Partial | P2/P3 | Persistent nation-specific core clans, atomic creation fee, invite/consent, leave/kick/disband, chief/vice/officer roles, roster, login fields and clan chat implemented. Member NP determines grade; Grade3 Caitharos trial, atomic key/fee promotion, chief-owned trial loot and persisted native cape purchases implemented. NP donation, rankings, symbols, alliances, siege and isolated trial instances remain. See [social systems](../re/social.md). | LAN-01, AUTH-01 |
| LAN-05 | Private messaging, friends and social chat routing | Partial | P2 | Normal/shout and membership-restricted party/clan chat work. Private target selection, friends and alliance chat remain absent. | LAN-01/02/04 as appropriate |
| ECON-01 | Player-to-player trade | Missing | P2 | No Rust0x30 dispatch/trade-session model. GS `TradeHandler.cpp` request/agree/add/decide/cancel/execute; two-sided atomic commit and cancellation matter as much as the window. This is distinct from NPC item exchange APIs. | LAN-01, AUTH-01 |
| ECON-02 | Player selling/buying merchants | Missing | P2/P3 | NPC shops work; no Rust0x68 player merchant dispatcher. GS `MerchantHandler.cpp` selling and buying flows. Buying merchant availability in native1453 must be verified before choosing that branch. Listings/reservation, atomic purchase and disconnect cleanup are required. | LAN-01, ITEM-03, AUTH-01 |
| ECON-03 | Letters, attachments and mailbox | Missing | P3 | No Rust0x6A shopping-mall/letter dispatch. GS `ShoppingMallHandler.cpp` routes letters; `LetterHandler.cpp` list/read/send/get/delete and database requests. Mail attachments require unique claim and transaction persistence. | AUTH-01; LAN-01 for live notifications |
| AUTH-01 | Password validation, login token binding and duplicate-session handling | Partial | P1 before LAN exposure | First-login registration and salted Argon2id verification are implemented, with locked per-server faction choice; see [accounts](../re/accounts.md). Rust `lobby.rs` still uses one-time IP association and rejects ambiguous same-IP pending logins; native token binding and duplicate live-session handling remain. Source GS `LoginHandler.cpp`/login server auth and `WIZ_KICKOUT` behavior. Implement SQLite-local authentication/session binding, not source SQL/ODBC infrastructure. | Native login-token evidence |
| CHAR-01 | Normal character deletion and appearance change | Missing | P2 | Source pre-world dispatcher includes `WIZ_DEL_CHAR`0x03 and `WIZ_CHANGE_HAIR`; Rust game dispatch creates/selects but lacks0x03 deletion flow. Manual prior SQL removal is not this feature. GS `CharacterSelectionHandler.cpp`, DB requests; ownership/confirmation and cascading records need tests. | AUTH-01 recommended |
| CHAR-02 | Name, nation, gender and job-change consumables | Missing / Unverified | P3 | GS `NPCHandler.cpp::HandleNameChange`, `CharacterHandler.cpp::NationChange/GenderChange/JobChange` have paths; no corresponding Rust dispatch/effects. Verify client support, item IDs, clan/party restrictions and need for each private service. | AUTH-01, ITEM-06 for metadata if needed |
| CHAR-03 | Server skillbar/hotkey save and reload | Implemented; native UI test pending | P2 | Native79/1 save and79/2 load,0..64 packed records, character-scoped SQLite persistence and reconnect restoration now exist. Source User.cpp/DatabaseThread.cpp/DBAgent and native0050FB40/0050F8F0 agree. See [skillbar](../re/skillbar.md). | None |

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
| OPTIONAL-03 | Helmet visibility, custom cape, Logos Shout and cosmetic services | Missing / Unverified | P3 | Source dispatch has `HandleHelmet`, `HandleCapeChange`, `LogosShout`; no Rust counterparts. Client version and private consumables determine applicability. Ordinary equipped cape rendering does not implement cape customization. | LAN-01, ITEM-06 if consumable state |
| OPTIONAL-04 | Mining, soccer and other source-private systems | Unverified | P3 | GS `User.cpp::HandleMiningSystem` contains mining/soccer; `HandleMiningStart` explicitly calls itself a mock-up and omits area checks. Their existence in this source is not proof of original1453 gameplay. Audit client UI/maps first or deliberately omit. | Explicit feature selection |

## Reference branches that are not complete implementation recipes

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

## Incoming packet coverage cross-check

This table covers the major gameplay entry-point families in the supplied
`CUser::HandlePacket` switch. Numeric IDs come from supplied `shared/packets.h`;
names remain evidence locators, **not** native1453 layout specifications. Several
source enums/widths already differed from the original client in prior work.

| Source entry point(s) | Current Rust coverage | Backlog |
| --- | --- | --- |
| Version/login, nation, new/list/select character | Bounded local implementation | BASE-01, AUTH-01 |
| Delete character, change hair | Absent entry points | CHAR-01 |
| GameStart, server index, hotkey/skill data | World bootstrap exists; optional server-index list is empty; server hotkey persistence absent | BASE-01, CHAR-03; native optional metadata audit if needed |
| Move, rotate, request user-in | Shared presence/movement and requested user-in; rotation remains | LAN-01 |
| Attack, target HP, magic process | Solo/NPC combat and supported effects | BASE-02, CLASS tasks |
| Chat, chat target | Normal/shout, party and clan; no private/alliance routing | LAN-05 |
| Regene, home | Town respawn/home; no XP loss/refund or bindpoint | PROG-01, WORLD-05 |
| Request NPC-in, zone change, warp lists | Imported NPC regions, selected gates/events | BASE-04/10, WORLD-07/08 |
| GM remote warp | No ordinary incoming GM warp handler | OPTIONAL-02 |
| ItemMove, NPC event/trade, item repair/remove | Working bounded inventory/NPC paths | BASE-07/08, ITEM-01..03 |
| BundleOpen, ItemGet | Working solo drop pickup | BASE-06, LAN-03 |
| PointChange, SkillPointChange, StateChange | Allocation/posture; not full group states | BASE-11, PROG-02, LAN-01 |
| Party, party BBS | Core party working; finder missing | LAN-02/03 |
| Exchange, merchant | Absent player trade/merchant; NPC exchange is different | ECON-01/02 |
| Quest, ClientEvent, SelectMsg | Ordinary quests/menus and scoped first promotion; current feedback defects under repair | QUEST tasks |
| ObjectEvent, ItemUpgrade | Gate/anvil subsets, normal upgrade | WORLD-05/07, ITEM-04..06 |
| Weather/time updates | No source dynamic controller/GM update path | WORLD-06 |
| ClassChange | Server promotion reply exists; incoming reset service absent | PROG-02 |
| ConcurrentUser, DataSave | No equivalent incoming request dispatch; authoritative actions save individually | Optional protocol audit; do not invent a gameplay gap from save-request absence alone |
| Knights process | Core clan membership working; advanced systems missing | LAN-04 |
| Operator | Existing local admin debug commands only | OPTIONAL-02 |
| Speedhack check | Packet recognized/logged; response semantics still TODO | AUTH-01/native heartbeat audit; no arbitrary disconnect timer |
| Warehouse | Working account storage/bank | BASE-07 |
| Friend | Absent | LAN-05 |
| VirtualServer | No multi-server handoff, intentionally outside single-host design | Not required |
| MapEvent | No-op in source; tile events already run on movement | BASE-10 |
| Event/Temple | No event controller | EVENT-03 |
| ShoppingMall/letters | Absent | ECON-03, OPTIONAL-01 |
| NameChange, King, Helmet, Cape | Absent | CHAR-02, EVENT-05, OPTIONAL-03 |
| Challenge, Rank, Siege | Absent | EVENT-01/02/04 |
| Rental | Reference handler itself incomplete | ITEM-06 / explicit investigation |
| Mining, LogosShout | Absent and native applicability unverified | OPTIONAL-03/04 |

Server-originating packets (gold, XP, death, user looks, effects, etc.) are not
all incoming-dispatch cases. Their absence/presence must be checked at the actual
event producer and native receiver. This matters directly for QUEST-01..03.

## Suggested next batches

1. **Leveling loop**: QUEST-04/05, PROG-02, ITEM-01/02,
   WORLD-03 and CLASS-05/06. This improves the user's current solo testing.
2. **World fidelity**: WORLD-01/02/03/04/05/07/08. Safe navigation and AI control
   are prerequisites for reliable monster spells and specialist quests.
3. **Two-client class testing**: AUTH-01, LAN-01, CLASS-02, LAN-02/03, CLASS-01.
   Shared player state is the dependency behind many otherwise working skills.
4. **Social/economy**: ECON-01, LAN-05, LAN-04, ECON-02/03 as desired.
5. **Optional endgame**: select EVENT/OPTIONAL IDs individually. It is reasonable
   to leave cash shop, mining, cosmetic/private additions or large war systems
   unimplemented for a singleplayer/LAN project.

Before closing a task: record exact source/data and native packet evidence, add
behavioral tests including refusal/replay/rollback as appropriate, verify with the
original client, and update this row with the concrete validated subset. Do not
mark a class or NPC category complete merely because one representative worked.
