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

1. Obtain the inputs listed in [reference data](reference-data.md).
2. Run `cargo run -p reko-serverd` once with valid client assets. It initializes
   SQLite and exits if the required LEVEL_UP rules have not been imported. This
   first missing-data error is expected; verify that no daemon remains running.
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
Ports are15100/login and15001/game. The server currently binds all interfaces
and advertises loopback. Public hosting and reliable simultaneous clients behind one IP remain
unsupported; see [accounts](re/accounts.md).

## Client and persistence

Use a separate local-testing client configuration with `IP0=127.0.0.1` in
Server.ini. Do not use scripts to modify the reference installation. With the
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
