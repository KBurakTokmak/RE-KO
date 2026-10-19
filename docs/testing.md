# Testing

Run from the repository root:

```powershell
$env:REKO_GAME_DIR = (Resolve-Path '../KnightOnline1453').Path
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --all-targets -- -D warnings
```

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
- Walk across regions and zone gates; verify nearby entities remain visible.
- Attack a neutral monster at range, observe retaliation, stop attacking and
  escape its leash. Check aggressive monsters, roaming, respawn and loot pickup.
- Accept multiple kill/item quests; verify per-kill updates, relog persistence,
  independent counters and immediate coin/XP rewards. Complete or abandon one
  and confirm other progress remains.
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
