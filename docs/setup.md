# Setup and operation

## Prerequisites

- Rust stable with the MSVC toolchain and Visual Studio C++ build tools.
- Windows 11 x64 for the supported original-client workflow.
- PowerShell and Python 3 for launcher, import and fixture scripts.
- A separately obtained, compatible1453 client installation and reference data.

Work from the repository root. `REKO_GAME_DIR` configures Rust asset discovery;
the fallback is `../KnightOnline1453`. Some Python/PowerShell helpers still
require the sibling directory layout in [reference data](reference-data.md).
An arbitrary client build is not interchangeable with the verified executable.

```powershell
$env:REKO_GAME_DIR = (Resolve-Path '../KnightOnline1453').Path
cargo build --workspace
New-Item -ItemType Directory -Path logs -Force
```

## Initial data

No game content or populated database is distributed. SQL files in the Rust
source define schemas; offline imports populate them from local reference files.
An empty database cannot provide the documented world/combat features.

### Existing installations and a friend's fresh installation

Code, schema and world content have separate versions. After pulling, run:

```powershell
python scripts/database_manager.py check
```

Exit code2 means preparation is needed; it does not modify the save. With players
logged out normally and the daemon stopped, run `python scripts/database_manager.py update`.
`Start-ServerWatch.ps1` also runs this update after building and before opening
the ports. Existing characters/accounts stay local to each installation.
Schema initialization uses the same Rust code as the runtime. Named content
migrations apply once; ordered migrations cover Delos/CZ/Ardream travel, combat-zone population and boss
pools, transformations, the bounded suppression repair and source zone metadata.
An update is staged on a disposable database, validated and published only on
success, with the prior save backed up under `logs/`. Missing inputs, conflicting
local routes, damaged saves and newer unsupported versions stop startup with an
error. They never trigger an automatic reset. Initial world imports below still require their original inputs. Do not rerun
them on an existing world merely to catch up with feature migrations.

For a machine without a database, first export a **world-only template** from a
provisioned installation:

```powershell
python scripts/database_manager.py export-template
```

Copy the resulting ignored `local-data/world-template.json` to the same relative
location on the other machine, alongside its separately obtained compatible
client. Then run the normal server starter. An absent database is created with
the current schema and template world rules; account registration and ordinary
creation defaults remain in code (level1,20,000Noah,300NP and the starter kit).
Templates select only an explicit list of world tables and required local combat
CSV inputs. They contain no accounts, passwords, characters, clans, privileges,
personal inventory, skillbars or grants. Imported world/training entities remain
world content. Keep templates/private source data out of Git. A clone without
either a template or the original reference inputs cannot reconstruct that data;
creation stops with instructions rather than opening an empty world.

An existing database never has its world tables replaced by a template. Bounded
migrations may read the needed source records from it when original map inputs
are absent. Refresh the template after adding imported content: export to a new
`--template` path, review it, then replace the previous local template intentionally.
The export command refuses to overwrite an existing file.


For reference-based updates rather than a shared current template, export the
required primary1453 JSON inputs read-only from your own SQL reference instance:

```powershell
.\scripts\Export-1453MigrationInputs.ps1 -Server '<reference-instance>' -Database '<reference-database>'
```

This writes only local MAGIC_TYPE8, the46 scroll MAGIC_TYPE6 rules, the eight
NPC-form audit rules and ZONE_INFO JSON. Other inputs are the original client,
Open-KO historical maps, supplementary2004 maps and existing reference-rules CSVs;
see [reference data](reference-data.md). Build `cargo build -p reko-tools` for the
original table reader used by imports. The manager runs the existing bounded
repair planners on its staging copy. Suppression repair retains its documented
owner-approved estimates; [rates/provenance](re/world-services.md#suppression-quest-loot-repair).
Missing inputs stop the whole update without publishing a partially migrated save.
A current world-only template supplies bounded migration rows when references
are unavailable on the other computer. Do not copy another developer's live save.

An intentional factory reset is available, with a backup of the old save:

```powershell
python scripts/database_manager.py reset --confirm-reset reko.db
```

This removes **all** accounts, characters and clans and rebuilds from the local
world template. It is never part of pulling or normal startup. `--db`, `--template`,
`--historical` and `--server` select other paths. Do not use a save database as a
template. A template checksum detects damage; accept templates only from trusted
sources, and use matching client/reference versions.

### Provisioning directly from original references

1. Obtain the inputs listed in [reference data](reference-data.md).
2. Run `target/debug/reko-serverd.exe --prepare-db reko.db` to create the schema
   without starting listeners or requiring game assets. This low-level command
   does not populate world data or back up an existing save; normal upgrades
   should use the database manager instead.
3. Keep the daemon stopped and import the reference rules:

```powershell
python scripts/Import-1453WorldRules.py --apply
python scripts/Import-1453Population.py --all-zones --apply
python scripts/Import-1453BossPools.py --apply
python scripts/Import-1453NpcAI.py --apply
python scripts/Import-1453MonsterCatalog.py --reference ../PV_1453/inspection/K_MONSTER.csv --apply
python scripts/Import-1453TravelQuests.py --apply
python scripts/Import-1453Experience.py --apply
python scripts/Import-1453ItemRequirements.py --reference "path/to/ITEM.csv" --apply
python scripts/Import-MapTriggers.py --apply
```

Replace `path/to/ITEM.csv` with your export location. The ITEM CSV must contain `Num`, `ReqLevel` and `ReqLevelMax`, exported from the
supplied1453 database. Boss grouping requires the population import first and
does not replace source spawn rows. Both new importers support a dry run by
omitting `--apply`.

The final trigger importer currently queries a local reference SQL Server;
it is an import-time dependency, not a runtime backend. Imports depend on the
supplied package/export schemas and are not a general installer for arbitrary
server releases. Review dry runs and script inputs before `--apply`. They back
up the save and refuse the running default daemon. Existing installations with
populated data do not need to rerun these imports for normal startup.
After initial imports, run the manager's `update` and export a world template.

The current combat loader expects `logs/reference-coefficients.csv`; supplementary
cure definitions use `logs/reference-rules/MAGIC_TYPE5.csv`. These are local
reference **inputs** despite their legacy directory name. Keep them locally;
do not delete the whole logs directory while cleaning captures. Missing
coefficients disable the optional combat catalog rather than supplying complete
combat from an empty database.

## Start and stop

Double-click `server_starter.bat` in the repository root to build and start
the background server and its client-startup watcher. Double-click
`server_stopper.bat` to stop this checkout's server/watcher and checkpoint
the save for copying (Python is required for the checkpoint). The scripts
resolve paths from their own location. They do not manage reference SQL services.
The start BAT runs `git pull --ff-only` before starting and cancels startup if
the update fails. The stop BAT attempts the same update after stopping, so a
network or Git error cannot prevent shutdown. Pull never forces a merge or
discards local changes; resolve any reported conflict before restarting.
Direct PowerShell invocation does not pull updates.
Both startup paths prepare the SQLite save through the database manager before
launch. Direct `cargo run -p reko-serverd` initializes schemas only; run the manager
first when content migrations are pending.

```powershell
# Background daemon, capture and client-startup observer
.\scripts\Start-ServerWatch.ps1
# Follow the current capture; Ctrl+C stops this viewer only
.\scripts\Watch-Server.ps1
```

The start script records the daemon PID in `logs/server.pid`. To stop that
specific daemon after checking its executable path:

```powershell
$serverProcess = Get-Process -Id ([int](Get-Content logs/server.pid))
$expectedExe = Join-Path (Get-Location) 'target\debug\reko-serverd.exe'
if ($serverProcess.Path -ne $expectedExe) { throw 'Unexpected executable' }
Stop-Process -Id $serverProcess.Id
```

Alternatively, `cargo run -p reko-serverd` runs in the terminal; Ctrl+C stops it.
Stop the daemon before replacing its Windows executable or importing game data.
Ports are15100/login and15001/game; the server binds all interfaces.
The starter uses automatic game addresses without prompting. Each login connection
receives the local IPv4 address it connected to: localhost, LAN or VPN. This uses
socket metadata, without scanning interfaces or echoing the client's source IP.
Clients configure the host's reachable IPv4 in their own Server.ini. Allow inbound
TCP15100 and15001 on the host firewall for the trusted LAN/VPN.

Only if automatic selection is unsuitable (for example NAT/proxy hosting), force
an advertised IPv4 explicitly:

```powershell
.\server_starter.bat -AdvertiseIp 192.168.1.50
```

Omit the option, pass `-AdvertiseIp auto`, or an empty value for automatic mode.
For direct `cargo run -p reko-serverd`, `REKO_ADVERTISE_IP` provides the same optional
override; absent/empty/auto means automatic. Explicit localhost means127.0.0.1.
An override applies to all login clients, so they must reach that advertised host
and use a consistent login/game route. Invalid/non-host addresses fail at startup.
The PowerShell starter clears inherited overrides in automatic mode and restores
the caller's environment afterward; nothing is saved in SQLite or committed.

Public hosting and reliable simultaneous clients behind one IP remain
unsupported; see [accounts](re/accounts.md).

## Client and persistence

Use `IP0=127.0.0.1` for a local server or the host LAN/VPN IPv4 for remote
play in the selected client copy's Server.ini. Do not use scripts to modify the reference installation. With the
server running, double-click `scripts/Start-LocalClient.cmd`. Its hash-checked
startup profile is described in [client startup](re/client-startup.md).
If the legacy XTrap child closes the client on this machine, prepare the isolated
copy once with `scripts/Prepare-LocalClient.ps1`, then use the same CMD launcher.
Preparation preserves the reference installation and requires roughly1.1GB of
local space. The copied profile disables the XTrap child; its API DLL remains.

`reko.db` is the authoritative save and imported world-rule store. Preserve its
WAL state when backing up a live server: use SQLite's backup API, or stop the
server and use a consistent database copy. Saves, backups, logs and local input
exports are ignored by Git. Test-fixture/grant scripts are optional offline
tools; they are not part of normal character creation or startup.
