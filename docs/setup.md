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
python scripts/Import-1453NpcAI.py --apply
python scripts/Import-1453TravelQuests.py --apply
python scripts/Import-1453Experience.py --apply
python scripts/Import-MapTriggers.py --apply
```

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
and advertises loopback. Public hosting and authenticated LAN operation are
not supported; see the authentication backlog.

## Client and persistence

Use a separate local-testing client configuration with `IP0=127.0.0.1` in
Server.ini. Do not use scripts to modify the reference installation. With the
server running, double-click `scripts/Start-LocalClient.cmd`. Its hash-checked
startup profile is described in [client startup](re/client-startup.md).

`reko.db` is the authoritative save and imported world-rule store. Preserve its
WAL state when backing up a live server: use SQLite's backup API, or stop the
server and use a consistent database copy. Saves, backups, logs and local input
exports are ignored by Git. Test-fixture/grant scripts are optional offline
tools; they are not part of normal character creation or startup.
