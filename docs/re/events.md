# Scheduled server events

`events.rs` is a pure state machine ticked once per second with server local
time; `event_runtime.rs` applies its actions (notices, zone eviction, spawns,
rewards). Timings, notices and win rules follow known 1453 server behavior
unless marked as an owner decision. Every event can be started and stopped at
any time with `op event` (see [chat commands](admin-commands.md)).

## Schedule data

Migration `011-world-events` imports the 1453 Forgotten Temple summon list
(`MONSTER_CHALLENGE_SUMMON_LIST`) and writes the owner schedule as
`EVENT_SCHEDULE` rows in `world_reference_rules`. Rows are JSON and stay
editable; local edits are never overwritten. Fields: `event`
(`temple|lunar|darklunar|siege`), `days` (`daily` or weekday list), `time`
(`HH:MM`, local), `enabled`, plus event rules (levels, chest, fee, capacity,
war zones, nation cap, NP per kill/death, invasion, victory rewards, siege
countdown/duration). Restart the server after editing.

| Row | Event | Default (owner schedule) |
| --- | --- | --- |
| 1 | Forgotten Temple 60-80, Blue chest 379156000 | daily 22:15 |
| 2 | Forgotten Temple 45-59, Green chest 379155000 | daily 23:30 |
| 3 | Lunar War, zones 61/62/64 by ISO week | Friday 20:00, 120 min |
| 4 | Dark Lunar War, zone 63, levels 35-59 | Thursday 20:00, 120 min |
| 5 | Castle Siege War (Delos) | Sunday 20:00 countdown, 10 + 60 min |

Missed ticks fire late; clock jumps over five minutes and server startup never
replay a start. **Events are not resumed after a restart** (owner decision):
players inside closed event zones are moved out on their next tick.

## Shared rules

- Zone eviction is a per-session policy (`Gates::exit`): Forgotten Temple
  without a running temple and Delos during the siege countdown go to Moradon;
  war zones without their war, the losing nation in a war zone, uninvited
  enemies in a homeland and Colony Zone/Ardream/71-73 during a war go to the
  nation castle. Administrators are exempt. Dead players move after respawning.
- Notices are native type-7 server notices with the 1453 resource texts.
- Rewards are written to `character_event_rewards` in one transaction, then
  delivered by the player's own session tick (items, Noah, XP, NP). If the bag
  is full or too heavy the reward stays queued in the save, the player is told,
  and delivery retries every minute, after relogs and after restarts. Nothing is
  lost (owner decision; 1453 drops it); Noah over the 2.1 billion cap also
  waits. A reward item missing from the client tables is logged and skipped
  without blocking later rewards.
- Temporary entity IDs (30000-32767) return to the pool 10 s after death or
  when the event despawns them, after an out packet. GM summons share the pool.
- Static NPCs used by a war (keepers, warders, monuments) are armed as combat
  entities with their existing IDs; the static entry is hidden while armed.
  Players cannot damage entities of their own nation, and the siege artifact
  only takes damage from clan members outside the owning clan. Kill credit
  falls back to the last attacker's nation and clan when the top damager has
  logged out; a monument is never left dead.
- Friendly skills follow the same hostility: no cross-nation buffs or heals in
  war zones or an invaded homeland, and none across clans in a siege battle.

## Forgotten Temple (zone 55)

Iris (13015) in Moradon runs a 1453-adapted menu. Entry requires the open
sign-up window, the bracket's levels, fewer than 33 players inside and the
100,000 Noah fee, then moves to (150,150). Timeline from the start time:
sign-up 5 min (notice 199), wave notice (200), first stage at 6:15, then one
stage every 45 s up to 56 stages. Only the first summon-list row of each stage
spawns, as in the 1453 loop; rows scatter within their radius of (126,126).
When every stage has spawned and every temple monster is dead, every living
player in zone 55 receives the bracket's chest, and the zone clears 30 s later.
Otherwise the run fails at 50 min: no chests, notice 202 after 30 s, monsters
removed and zone cleared after 230 s. Death or `/town` in the temple returns to
Moradon (revived, no XP loss). Pre-start notices (198) run 5 min and 20 s
before. The 1453 client texts still advertise the original hours.

## Lunar War and Dark Lunar War (zones 61-64)

Entry is through the homeland Lunar Gate tiles while the war is open, at levels
35-83 (Dark Lunar 35-59), at most 150 per nation; the losing nation cannot
enter. Opening arms the zone's keepers and warders (half HP, a fifth of armor)
or the seven Nereid monuments in zone 64, and evicts Ardream (and Colony Zone
71/73 for the Lunar War). Status notices run every 5 min (2.5 min in zone 64).

- Zones 61-63: killing three enemy keepers/warders wins at once; kills are
  announced with the top damager's clan and name.
- Zone 64: a monument captured by a player changes nation and look; +2 points
  per capture, +10 while holding 7 or more. Holding 7+ against 0 at 15 min wins.
- At half time without a winner: NPC kills (61-63) or monument points (64),
  then player deaths; a further tie is a draw that closes the war.
- NP per war-zone kill: +64 / -50 (1453 code defaults, owner editable).
- Result: winner notices, losers leave the zone, and online winners in their
  homeland or the war zone receive 100,000 Noah and 5,000 XP (1453 victory
  bonus). The Lunar War then opens the loser's homeland: winners may enter by
  the war-zone exits, homeland PvP and guards turn hostile (`set_guard_invaded`)
  until the war closes. The Dark Lunar War closes on its result.
- Close at 120 min (notice 133); 60 s later all war zones and invaders clear.

The 1453 result code looks up a hard-coded online character before kicking and
invading; RE-KO does not depend on any character.

## Castle Siege War (Delos, zone 30)

Countdown 10 min with notices at 10/3/2/1 min while Delos is closed. The battle
lasts 60 min: the Center Artifact (541) is placed on its original Delos object
position, players of different clans are hostile in Delos, and the clan whose
member destroys the artifact becomes the persistent owner (`castle_siege_owner`,
notices 179/194, native `WIZ_SIEGE` owner update). The battle continues to its
end, which names the owner. Registration follows 1453 (none).

## Pending

- Border Defence War: infrastructure only; owner rules are pending (rooms,
  scoring, rewards).
- Snow War, King events, XP/Noah bonus events, war captains/commander fame,
  `WIZ_BATTLE_EVENT` result UI, Delos tax and siege registration.
- Keepers and monuments do not fight back; artifact respawn/recapture, the
  Delos re-positioning after capture and the owner packet on Delos entry.
- In-game checks: artifact visibility (no NPC_Looks model), temple respawn zone
  change while dead, monument look change, and type-7 versus type-8 notices.
