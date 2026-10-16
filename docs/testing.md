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
details local. Administrative `hurt N` is allowed only by character_privileges;
admin status does not bypass normal costs or grant immunity.
