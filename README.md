# RE-KO

RE-KO is a Rust reimplementation of the Knight Online **1453** server, aimed at
preserving the older game experience for singleplayer and small LAN sessions.
The current implementation works with the original 1453 Windows client.

The long-term goal is a reusable server library for both a standalone LAN server
and an in-process singleplayer host, followed by a new client. Today, development
focuses on the standalone server and core gameplay compatibility. A replacement
renderer/client and an in-process game host have not been implemented.

## What works today

- First-login password registration, password validation, permanent per-server
  nation selection, character creation and persistent characters.
- Movement, sitting, location persistence, Town and zone gates, including
  walkthrough Eslant teleport triggers.
- Inventory, equipment, buying/selling, repairs, account storage, and normal anvil
  upgrades using imported recipes and rates.
- NPC population and supported shop, storage, dialog and quest interactions.
- Imported monster populations, roaming, aggression, ranged retaliation,
  chasing, physical attacks, regeneration and source-based respawn timers.
- Monster loot boxes, drop tables, item pickup, coins, solo XP and leveling to 80.
- Ordinary kill/item quests, rewards, progress notifications, level10 first job
  change, and up to **40 concurrent accepted quests** with separate kill counters.
- Combat and many Rogue, Warrior, Mage and Priest skills, including timed buffs,
  healing, drains, multi-arrow attacks, area spells and supported NPC debuffs.
- HP/MP potions, natural regeneration, weapon durability and recovery updates.
- Administrative test fixtures, packet capture and a local-client startup profile.
- Admin-only 10,000-damage basic attacks, monster protection and native GM
  collision bypass, monster summoning by ID/name; per-character skillbar save/load.

These are working subsets, not a claim of complete retail behavior. Some NPC
services, specialist quests, skills and multiplayer mechanics remain incomplete.
Terrain pathfinding, monster spells, parties, trading, clans and full PvP are
among the remaining systems. See the [feature backlog](docs/design/1453-feature-backlog.md).

## Run and develop

The supported development target is **Windows 11 x64**, using Rust stable
(edition 2024/MSVC). Windows launcher/import tools also need PowerShell and Python 3.

```powershell
$env:REKO_GAME_DIR = (Resolve-Path '../KnightOnline1453').Path
cargo build --workspace
cargo test --workspace
cargo clippy --all-targets -- -D warnings
```

Read the [setup guide](docs/setup.md) before starting a new database. A clone
alone does not contain the original client, game assets, reference databases,
world data exports or an existing save. Several integration tests require those
local inputs and skip when they are absent.

With local data prepared, run from the repository root:

```powershell
.\scripts\Start-ServerWatch.ps1
```

You can also double-click `server_starter.bat`; use `server_stopper.bat` to stop
the server and its startup watcher.

This starts the server in the background with login on TCP 15100 and game traffic
on TCP 15001. For an attached terminal session, use `cargo run -p reko-serverd`.
The daemon stores characters and imported rules in local `reko.db`.

**Trusted networks only:** passwords are validated, but the legacy game-session
handoff still uses a temporary IP association and the default listeners bind
all interfaces. Do not expose this development server to the public internet.
See [accounts](docs/re/accounts.md) for first-login registration and migration.

## Repository layout

| Path | Purpose |
| --- | --- |
| `crates/reko-server` | Authoritative gameplay, protocol handlers and SQLite persistence |
| `crates/reko-serverd` | Standalone server executable |
| `crates/reko-common` | Shared constants and game installation discovery |
| `crates/reko-assets` | Original table reader and cipher |
| `crates/reko-tools` | Asset inventory/table inspection CLI and local startup helper |
| `scripts` | Server/client launch, offline imports, diagnostics and test setup |
| `docs` | Current setup, architecture, protocol findings and feature backlog |

## Data and scope

Original client tables and verified native packet behavior take priority.
Available 1453 server references guide world data and mechanics; Open-KO1298/9
and later2086 references provide corroboration where appropriate. Version
differences and intentional local rules are recorded in the documentation.

This repository contains no original game binaries, assets, private-server
packages, database backups, personal saves or packet logs. Obtain any required
reference material separately. The project is independent of the original game
and does not include permission to redistribute its files.

Start with the [documentation index](docs/README.md) and
[contribution rules](AGENTS.md). The Rust workspace declares the MIT license;
see [LICENSE](LICENSE).
