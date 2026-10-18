# Administrative combat and native GM movement

The sole privilege source remains `character_privileges.is_admin`, scoped by
account/character slot. Names, account names and client authority requests cannot
grant this role. No new per-feature permission flags or automatic grants exist.

## Basic attacks and monsters

Successful admin basic R attacks apply 10,000 physical damage, clamped to the
target's remaining HP. Life, attack delay and target validation still apply.
Basic attacks use weapon reach plus half the sum of the player's original
skeleton radius and the target's radius scaled by its spawn size. This applies
to normal and admin characters equally, with the owner's20percent multiplier,
3m minimum and0.15m rounding tolerance; see [combat](skills-combat.md).
The old fixed2m body allowance rejected native large-boss R attacks (observed
on Troll King). The supplied source's generous15m base is not used as a substitute
for model geometry. Skill range handling remains separate. Admin hits still
reject targets beyond their bounded model-based distance.
All weapon classes can use this testing override; skill damage is
unchanged. The normal monster hit/death pipeline handles HP, damage contribution,
loot, XP, quest credit, respawn and weapon wear. Ordinary characters retain the
existing formulas and weapon restrictions.

Source: supplied1453 `GameServer/Unit.cpp::GetDamage` has a GM-only normal-hit
override (`m_nGameMasterRHitDamage`). 10,000 is the owner's requested testing
value. `AIServer/Npc.cpp` excludes GMs during target search and attack handling;
Open-KO's corresponding AI is the corroborating reference.

Each character tick refreshes the cached role from SQLite before AI observation.
Admin players are ineligible for both proximity aggro and damage retaliation;
existing targets are released. The delivery path also discards queued monster
strikes after promotion, preventing a previously queued hit from doing damage.
Revocation restores normal combat without reconnecting. Missing roles default
to normal behavior. These policies do not block explicit admin `hurt N` testing.

## Native collision bypass

The world-entry authority byte now maps tagged admins to GM0 and normal players
to1. Native1453 `006A0BC8` stores it at player+0x4F8. Collision checks read that
field: `0047D30C` skips the player collision block for GM0, and `0047DDEB`/
`0047DDF3` bypass a shape-collision rejection for GM0. Additional slope checks
in the same routine are similarly authority-gated. Open-KO
`Client/WarFare/PlayerMySelf.cpp::CheckCollision` corroborates the GM exclusion
from object/player and slope checks (`AUTHORITY_MANAGER=0`).

This activates the existing client behavior through the native snapshot; no
EXE changes or process-memory patches. Server speed/distance limits remain in
place. It is not free flight or permission to change zone IDs. The native
authority is sent on entry, so role changes need reconnecting to update client
collision behavior, even though server combat permissions refresh live.

Tests cover armor-independent admin damage, delay/range checks, remaining-HP
clamping, revocation, ordinary hits, passive/aggressive AI exclusion and queued
damage suppression. A packet test verifies the authority byte is the only
changed world-entry field. Native scenery walking still needs gameplay testing.

## Summoning monsters

Use normal/all chat on a database-tagged admin character:

```
spawn 803
spawn Rabid Bandicoot
mobfind harpy
mobfind 2 harpy
+monsummon 2201
```

Use bare `spawn`/`mobfind` or `+monsummon` in this client. Open-KO UIChat forwards
slash text to GameProcMain::ParseChattingCommand instead of ordinary chat; its
unknown-command branch sends nothing. Observed1453 captures likewise contain
the bare attempts but no `/spawn` attempt. The server also accepts slash forms
if delivered, but that does not register a new local client command. Prefix-only
server recognition previously let bare attempts fall through to ordinary chat. A socket
regression now routes that exact captured packet through the game dispatcher
and checks the actual spawn event and private notice.

`spawn` accepts a prototype/database ID or case-insensitive name (spaces allowed;
quotes optional). Exact names take priority; a unique substring is also accepted.
Duplicate names show matching IDs without spawning anything. `mobfind` searches
all containing names,12 results per page; an optional page number comes first.
Results and errors are private native chat notices. IDs are prototype IDs, not
the temporary entity IDs shown after summoning. `+monsummon` is a source alias.

The supplied1453 ChatHandler::HandleMonsterSummonCommand and
GameServerDlg::SpawnEventNpc supply the reference: summon one monster at the
caller's position in their zone, without respawn. RE-KO uses MONSTER_TEMPLATE
rows imported from full K_MONSTER, not just prototypes with ordinary map spawns.
The importer copies appearance, size, stats and resistances; existing AI,
MONSTER_REWARD and drop tables continue to determine behavior/rewards. Existing
loot rates are untouched. The loot level lookup additionally fills otherwise
missing prototype levels from the catalog, preserving existing map-derived values.

Original-client model/skeleton asset availability and template validity are
checked before insertion.
Private-source entries without a1453 model return an error rather than spawning
an invisible replacement. No client files are modified. Summons join the normal
region event, combat, death, loot, XP and quest paths. They exist until killed or
server restart; they never alter permanent spawn rows. Entity IDs30000..32767
skip reserved map/NPC IDs and existing entities; dead IDs are not reused within
the same run, avoiding stale-target/loot identity collisions. Exhaustion returns
an explicit restart notice. Permission is rechecked in SQLite for every command.

Offline setup: export supplied1453 K_MONSTER as CSV, then run
`python scripts/Import-1453MonsterCatalog.py --reference <K_MONSTER.csv> --apply`
with the daemon stopped. The importer backs up the save and only upserts
MONSTER_TEMPLATE rows in world_reference_rules. The CSV remains external.

## Natural boss status

Type `bossstatus` in All chat to list this zone's natural rare bosses, or
`bossstatus Talos` to filter by name. Admin-only notices show the selected spawn
coordinates while alive, or the remaining six-hour timer after death. Each server
start resets timers and spawns one boss per configured boss/zone pool. Manual
`spawn` summons are separate one-life entities and do not change these pools.

## Coordinate teleport

In All chat, type `teleport 445 555` (or `tp 445 555`) to warp within the
current zone using displayed map X/Y coordinates. Commas and decimal coordinates
are accepted, for example `teleport 445.5,555`. The second map coordinate is the
server's horizontal Z; height is resolved by the native warp/terrain behavior.
Protocol coordinates support0..6553.5m at0.1m precision; this does not guarantee
every destination lies on playable terrain.

Each request rechecks the character's database admin role and requires a living,
entered character outside zone loading. Save failure leaves the current location
unchanged. The shared Town relocation path saves before native WIZ_WARP0x1E,
clears stale cast/NPC/sitting/movement state and immediately refreshes regions.
Supplied1453/Open-KO `User::Warp` is the packet/coordinate reference. Use bare
commands: the original client may intercept unknown slash commands locally.
