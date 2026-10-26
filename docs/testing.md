# Testing

Native reverse requests use5B/7 with the same55-byte mode/NPC/ten-item layout;
verify +7 Shard conversion through that actual client path and source recipe,
not just a forged normal5B/2 request. The focused test checks native7 response,
preview preservation and one-time consumption. For moving combat, test Spike
108655 while running past a target with Light Feet, plus R; stop outside range
and attack a target sideways outside the validated path to confirm refusal.
Neither case may spend mana on a refused skill. The Win32 tooltip harness hides
the tooltip before every draw, matching the actual inventory render lifecycle.

Inventory regressions include consuming a Trina at the anvil, depositing and
withdrawing another nonstackable item, and partial scroll-stack transfers. The
native5B/45 replies must own these icons without duplicate3D updates. Verify that
no ghost icon remains after closing/reopening the bag or bank. Reverse conversion
tests cover preview, source poison +7, requested plain +7, Exceptional Shard +7,
replay refusal and an ineligible Cleaver preserving its scroll. Native-client
visual checks remain necessary. Test `/autoloot` on/off in ordinary chat, owned
versus other-player/distant boxes, a full bag, and movement away from an NPC
service context. Login starts with autoloot off.

The local UI cache has an additional Win32 calling-convention/behavior harness;
see [client startup](re/client-startup.md#local-inventory-ui-fixes). It checks real
x86 code execution in an isolated test process, not the game's measured FPS.

For the suppression reward/merchant regression, hand in the three regular and
one boss certificate to Fargo as a rogue. Confirm both source weapons appear
in the vacated bag slots, with received-item notices. Then open Moradon's
[Scrolls] Charon and confirm the bag icons transfer to the shop without a crash.
Repeat the NPC interaction to confirm no duplicate payout. Automated socket and
SQLite coverage verifies the bundle, ordered icon removal/creation, notices,
unchanged unrelated bag items and single completion; native rendering still
requires this manual client check.

Run from the repository root:

```powershell
$env:REKO_GAME_DIR = (Resolve-Path '../KnightOnline1453').Path
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --all-targets -- -D warnings
python -m unittest discover -s scripts -p test_database_manager.py -v
```

Database-manager regressions use synthetic world rows and no private fixtures.
They exercise preservation, repeat application, rollback, fresh creation,
world-only template filtering, explicit reset backups and future-version rejection.
Also test a disposable fresh database using a real local template before sharing
it; verify empty account tables and normal registration/creation defaults.

Some tests skip without original client assets, coefficient exports, supplementary
Lua or imported fixture databases. Broad NPC/quest audits can take about two
minutes with local data. A clean clone passing tests without those inputs does
not exercise the same coverage as the fully provisioned development workspace.

The suite covers framing and crypto, character ownership/persistence, native
packet widths, equipment moves, shops/storage/upgrades, loot replay protection,
AI and roaming, skill resource rollback, cooldown/regen timers, XP thresholds,
ordinary quests and the40-active-quest boundary. Use isolated fixtures when
adding tests; never mutate the live save as a test harness.

Useful original-client regression passes:

- Create a normal character; verify equipped starter gear,20,000Noah,300NP and
  location persistence. At level10 use Kaishan's first job change.
- Buy, equip, swap and sell items; verify pants, both rings, hand conflicts,
  inventory icon removal, durability loss and sundries repair.
- For potion economy overrides compare actual deducted Noah and server total
  weight, not unchanged client item text. Soul costs11250, Wisdom/Favors5250,
  Speed+1000; other shop HP/MP potions retain20percent of original price and all
  ten recovery-potion weights halve. Repeated migration/start must not discount again.
- Deposit/withdraw Noah at an Inn; check immediate wallet/bank balances and
  persistence after relog. Store an existing scroll stack and equipment on page8;
  reject merging separate nonstackable stacks. Captured money packets use255
  page/slot sentinels, which must remain invalid for item transfers.
- On an admin, `zonechange aradon` should privately suggest Moradon21 without
  moving; `zonechange 71` should load Colony Zone. Verify a normal/revoked
  character cannot use the command, and ordinary gates retain their entry rules.
- Walk across regions and zone gates; verify nearby entities remain visible.
- Attack a neutral monster at range, observe retaliation, stop attacking and
  escape its leash. Check aggressive monsters, roaming, respawn and loot pickup.
- Accept multiple kill/item quests; verify per-kill updates, relog persistence,
  independent counters and immediate coin/XP rewards. Complete or abandon one
  and confirm other progress remains.
- In MSS II, loot three hunting certificates from normal mobs and the Grape
  Ripper certificate from its boss. Check the corresponding water/marble in
  MSS III. Verify ordinary full-bag/range/party/replay rules still apply. Defense
  armor is rare; use the deterministic importer/loot regression rather than
  expecting a dagger helmet on a short run. Selected +1 variants use original
  extension541; the estimates and relative part/class weights are documented.
- In both suppression zones, destroy the room gates and confirm their visible
  barriers disappear. Check the own-nation exit portal, rejection of a distant
  click/forged destination, free travel and preserved quest/items. Defeat the
  natural boss by direct damage or DoT, collect its certificate during the
  60-second chat countdown, then verify automatic home travel. Relog during the
  countdown to check that it does not restart; early portal departure must not
  cause a later unexpected warp. Native visual/collision checks require the client.
- Exercise class skill costs, effects, cancel/expiry, potion recovery, standing
  and sitting regen. Test arrows at close range and mage ground AoE after moving.
- Use the local startup launcher and leave the client idle beyond the former
  six-minute watchdog exit window.

For skill reports record the exact skill, target, distance, HP/MP change,
cooldown and visible result. For unexpected exits distinguish deliberate close,
socket disconnect and process termination. Keep captures and test character
details local. Administrative `hurt N` is allowed only by character_privileges.
Tagged admins also have fixed R damage, monster protection and native GM
collision behavior; normal skill costs still apply. See [admin testing](re/admin-testing.md).

## Party, clan and large-drain checks

- On an eligible Grade3+ training clan chief, open the nation's knight clerk and
  select knight privileges. Confirm the native1453 dialog opens without the
  former missing-talk crash. Start the trial; verify zone54 and transfers for
  online mainland clan members, including solo entry. Kill Caitharos and pick
  up its Key of Honor as chief. Use Judge to return home and claim promotion
  with the key and10000000Noah. Check one fee/key consumption, rank/cape after
  relog, and rejection of a repeated claim or non-chief request.
- At Habice, select a cape allowed by the clan grade. Verify its original table
  price, persistence and appearance. Check that insufficient funds, a distant
  merchant or a training clan cannot purchase, and reselecting the current cape
  does not charge again. Keep existing saves unchanged merely to reach this gate.
- Type `quests`, `quest <ID>` and `questcancel <ID>` in ordinary chat. Check
  private pages, original descriptions and current kill counters. Abandon one
  quest while another remains active; only its own counters reset. Completed
  quests/internal flags cannot be abandoned. Check suppression return as well.

- On a high-HP target, cast10percent twice and then5percent: HP loss uses the
  remaining HP each time and can exceed32767. Large-hit notices give full totals.
- Create a clan from the home Inn Host as a level20+ character with500000Noah.
  Check the deducted fee, roster, own clan display and persistence after relog.
  Invite a same-nation character; reject an enemy-nation invitation.
- With two similar-level characters, invite/accept through native party controls.
  Check the roster, HP/MP updates, private party chat and shared nearby XP/quest
  kills. Pick up coins and multiple items to check level shares and item rotation.
  Move a member beyond50m and verify they do not receive kill rewards.
- Test leave/kick, leader promotion and disconnect. A departing leader disbands
  the party; clans persist. Existing solo loot, shops and storage remain checks.

The supported local launcher permits one game instance. Manual two-client
coverage therefore needs a second PC/client with a different account. Complete
one account's login/game connection before logging in another behind the same IP;
native token binding for simultaneous pending logins remains unrecovered.

For native NPC synchronization, verify walking and stopping targets with R and
moving Assassin skills after relogin. The deterministic interpolation regression
models1453 MoveTo at2/4/8m/s and checks arrival cancellation and visual melee
distance; it does not simulate packet loss or stalled rendering. The Win32 tooltip
harness covers hundreds of cursor moves, screen-edge anchoring, per-frame hiding
and immediate item/stat invalidation. The additional Win32 melee harness checks
movement-before-R ordering, preserved attack arguments/stack, moving forecasts,
changed stationary positions and no extra movement for unchanged positions.
The socket/SQLite stale-stop regression rejects repeated out-of-range R despite
a claimed short packet distance, rejects a forged position, persists a valid
final stop before allowing damage, and retains attack cooldown enforcement.
Native owner testing reports R appears repaired with the isolated profile.
Additional item combinations, LAN latency and broader combat feel remain checks.

For multiplayer presentation, use two nearby normal/visible characters. Check both
HP bars by selecting each other and a merchant/monster; observe HP changes outside
a party too. Test R, targeted magic, ground AoE, bow flight and level-up effects on
both screens, movement/backward/speed changes and stopping without a position
change. Summon a party member within one region, across regions and after relog;
both must see each other immediately. Repeat with a hidden GM to verify privacy,
not to globally reveal admins. Monster HP bars should use each committed snapshot
without an extra client interpolation delay. Preserve party XP and loot routing.

Keep a partially elapsed Speed+ potion and HP/AC scroll active while using gates
between Moradon and the home castle. Confirm effects return on load completion,
remaining times continue (not restart), resources/pack charges stay unchanged and
recast remains blocked until normal expiry. Compare after relog as well.

Cross-zone party check: leave Luferson for Moradon as an ordinary member and as
leader. The traveller must disappear from the server party as well as the UI;
remaining members keep a valid leader/loot cursor if two or more remain. Return
without reinviting: no shared rewards or party summon eligibility should return.
Same-zone Mage summons must preserve the party.

## Native player trade

- With two nearby characters, request trade and decline once; verify both can
  continue normally. Then accept, offer Noah plus equipment and a partial stack
  from each side. Confirm one side first: nothing transfers. Confirm the other:
  verify items, counts, durability and balances immediately and after relog.
  Leave the window open while adding both items and Noah across several periodic
  timestamp checks; it must stay open. With a third player, try competing requests
  to either participant during pending consent and active exchange: reject the
  extra requests while preserving the original pair and its offers.
- Fill both bags and swap one whole item each: outgoing slots must make room.
  Try a gift into a full bag, an overweight receipt and a currency-cap overflow:
  both sides must receive failure and keep their original items/coins.
- Cancel after offering items/Noah; also walk away, teleport and disconnect.
  Verify restoration and that a later fresh trade works. Offers cannot change
  once either side confirms. Check normal party loot while a trade is open:
  the trade closes before the external item/Noah update, without losing rewards.
- Have two players attack one monster separately and with overlapping DoTs.
  Each sees only their own damage in logs, while both bars show identical HP.
  Cross-zone travel removes the traveller from the party; same-zone summons
  retain it. Active Speed+ buffs must survive both zone transitions.

Automated trade checks use two real TCP socket pairs through the game dispatcher
and SQLite rollback injection, including full-bag swaps, final weight/coin/ownership
failure, stack splitting, whole charged packs, cancellation and replay rejection.
Native UI rendering still requires the two-player checks above.

For Mage overlap, cast Fire Spear from range and immediately cast Ignition while
the spear travels. Both must damage in either arrival order, including with a
transformation. Repeat while moving after launch: the paid spear keeps its launch
origin, and the new spell still uses its own range/resource/cooldown checks.
