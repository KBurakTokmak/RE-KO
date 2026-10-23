# Original-client startup

## Compatibility with unchanged clients

The owner prefers the original client for all players and server-side fixes
where possible. The isolated profile is a local workaround, not a prerequisite
for the server protocol or evidence that every original client needs these fixes.
Existing local remedies are approved to remain. Before a new client file or
process-memory change, explain why it is needed and which files/areas it affects,
then wait for explicit owner approval.

| Local workaround | Server-only equivalent |
| --- | --- |
| Inventory tooltip cache | No equivalent through the recovered protocol. The invalidation/layout loop runs inside the client even without a server update; server stats must not be falsified to avoid drawing it. |
| XTrap child replacement and watchdog shutdown | No established server-only equivalent. The reproduced termination originates in local startup/watchdog code; no verified game-server message replaces these local steps. Compatibility may differ between machines. |
| English transformation list names | The native list reads local Skill_Magic_6 column2. Its recovered open packet sends a list skill ID, not replacement strings. A separate server-authored selection interface would be a different workflow, not a repair of the existing list. |
| Final-position MOVE before R | The server already validates movement forecasts and original model reach. An omitted final coordinate cannot be uniquely reconstructed from the attack's target and surface distance. A wider range exemption is not an equivalent correction. Further server-only compensation needs evidence and bounded validation. |
| `/autoloot` slash routing | Ordinary chat `autoloot`, `autoloot on` and `autoloot off` already reaches the same server handler without this hook. The unchanged client's unknown-slash-command branch prevents the slash spelling from reaching the server. |

Gameplay, inventory transactions, costume equipment/bonuses, reverse upgrades,
monster drops and transformation rules remain server-side. The protected
KnightOnline.exe is not patched on disk by the supported profile. Tooltip,
autoloot routing and final-position synchronization are temporary process-memory
hooks; English names and helper executables affect only the isolated copy.
Do not disable working local remedies and claim stock-client equivalence without
testing their replacements. The original sibling installation remains read-only.

The repeated idle/combat exit was traced to explicit self-termination, not an
unhandled game exception. A debugger caught TerminateProcess(self,0) at uptime
about356seconds, return address0078D93B. Watchdog0078D8C0 waits on event009E02F4
with120000ms timeouts; a missing heartbeat flag009E02F8 reaches a delayed exit.
Handler0078D990 supplies that flag for status0x5A004. This establishes the specific
timed-exit cause, not the cause of every future crash.

`scripts/Start-LocalClient.cmd` invokes the supported standalone startup profile.
`local_client.py` checks the exact executable SHA256 and unpacked watchdog
signature, duplicates the event handle and signals the watchdog's own shutdown
path. The watchdog step itself does not write process memory. The optional local
UI fixes below use verified temporary hooks; original files remain unchanged.
The server watcher can apply the same profile to an exact newly created
process within30seconds; late attachment is refused and a mutex prevents races.

The verified EXE SHA256 is
`9513ecdf29e99bfee307fa8dc93ef64bf616d77d475d57fde906107f32169bb0`.
Other builds are rejected. The helper requires Windows, Python3 and local
Server.ini configuration. Existing game instances must be closed before using
the explicit launcher. No permanent EXE dependency removal has been performed.
Startup debugger attachment and
bootstrap-skipping experiments were unsuccessful and are not supported launchers.

## Isolated local profile

The heartbeat shutdown alone does not stop the separate XTrap executable from
closing the client during initialization on some machines. A reproduced launch
reached the watchdog-ready marker, then exited with code0 roughly12seconds later
and displayed an XTrap Control window. Its exact error code was not captured.
Earlier machine logs recorded XTrap startup crashes, so that earlier success did
not establish compatibility with a working XTrap child process.

Run `scripts/Prepare-LocalClient.ps1` once with Rust available. It copies the
original installation to ignored `.local/client-1453`, builds the existing
XPatch success helper and an inert `reko-local-guard` child, and installs those
only in the copy. The local Server.ini targets127.0.0.1. An existing destination
is refused to preserve its configuration. Allow roughly1.1GB for this copy.

`Start-LocalClient.cmd` selects the prepared copy when its profile manifest is
present. Python verifies the original client hash and both helper hashes before
launch. The startup observer accepts only the original or this local directory,
passing the observed directory explicitly; direct launches still need the
watchdog shutdown. Use the CMD entry point for reliable local initialization.

The original KnightOnline.exe and XTrapApi.dll remain unchanged. XTrapApi.dll is
still required by this profile; this is not a fully stripped client. Removing
the copied XTrap directory or DLL is unsupported. The inert child performs no
checks, communication, or gameplay work. The existing watchdog shutdown is still
necessary. A local trial reached login, entered the world and remained connected
beyond400seconds, exceeding the former356-second termination window. Longer
sessions and unrelated client defects still require separate validation.

Sessions beyond the former exit window were validated, including a21-minute
idle run closed intentionally by the tester. A later run exceeded an hour.
Future exits still need their own process/packet evidence. Keep raw dumps,
debugger traces, protected binaries and generated decompilation outside Git.

## Local inventory UI fixes

The isolated profile also runs `local_client_tables.py` before launching.
Original Skill_Magic_6.tbl has English names in column1 and legacy non-English
display strings in column2. For the46 Transformation Scroll472xxx forms, the
local copy uses the existing English bytes in column2. Every other raw cell,
row/header and numeric gameplay field is preserved and checked. The source
installation is read-only; an unknown local customization is refused. A local
backup/receipt stays under ignored logs. Preparation and the launcher both apply
this idempotently; replacing the copied table is atomic. Fully restart the game
through Start-LocalClient.cmd to load these names. Relogging does not reload TBLs.
`python scripts/Test-LocalClientTables.py` checks the byte-preserving repair,
cipher round trip, idempotence and truncated input without changing a game file.

The isolated profile enables `client_tooltip_cache.py` after unpacking and the
watchdog shutdown. Native inventory rendering calls00506CD0 at004BCA69. The
tooltip compares raw mouse coordinates with its cache, but00500C19 stores the
coordinates after00506D20/00506D24 add two pixels. Thus unchanged hover repeatedly
rebuilds the text/layout, particularly expensive for wearable equipment. The
content cache at this inventory-only call site retains native drawing and
placement. Inventory Render004BC68A calls DisplayTooltipsDisable each frame;
00500876 also clears the native previous-item pointer. Hidden state therefore
cannot invalidate content. The hook restores native visibility on a hit.

An earlier sixteen-frame refresh still caused expensive text/layout rebuilds,
and treating mouse coordinates as content keys rebuilt again on every cursor
movement. Live timing of that version recorded117builds for1864draws, with
about23.37billion CPU timestamp cycles in native preparation/render on misses
versus1.56billion in cached rendering. These are CPU-cycle totals, not FPS.
Version3 replaces periodic refresh with exact snapshots of the native player's
class/race/level fields004E0..004F7 and extended stats00848..00927, relative to
`[009DD3AC]`. Those ranges cover the builder's equipment requirement/stat reads,
including00922. Item pointer, table pointers, count, durability and display flags
also invalidate immediately. A missing player/item uses the native path.

Cursor movement now calls native SetPos (virtual+24), moving existing children
without regenerating their strings/fonts. Screen-edge anchoring follows
005008A0 with its two-pixel input adjustment and26-pixel horizontal offset;
viewport dimensions are at008A4944/008A4948. Native item tables remain immutable.
Unrelated tooltip windows are outside this inventory-only hook. Writable data
contains cumulative draw/build counts and timestamp-cycle totals for native
preparation and cached rendering, allowing local performance diagnosis without
per-frame logging. The owner reported the hover slowdown resolved after live
application; this does not establish performance on every item or machine.

The same helper separately routes only `/autoloot` (with a word boundary) from
native command handling: the verified UI chat slash branch is005762F5. It
continues at0057630B to send the unchanged text as ordinary chat instead of
discarding an unknown local command. All other slash commands retain the native
branch to005762FA. This is required for the owner-requested slash syntax.

All hooks require the exact original EXE hash, isolated profile path and expected
unpacked instruction bytes. Code runs in a separate executable page, cache data
in a writable page. Target threads pause only during the five-byte replacement;
original page protection and instruction cache are restored. Startup failures
leave the normal client usable and are logged. Neither EXE on disk is changed.
Set `tooltip_cache`, `autoloot_command` or `melee_sync` to false in the local profile JSON to
disable the corresponding hook on future starts. For a running client:

```powershell
python scripts/client_tooltip_cache.py --pid <client-pid> --remove
python scripts/client_tooltip_cache.py --pid <client-pid> --command --remove
python scripts/client_tooltip_cache.py --pid <client-pid> --melee-sync --remove
```

Version3 receipts remain in ignored logs; installing over an older recognized
receipt first removes its hook. Removal restores the original bytes, leaving
the tiny allocation until process exit to protect in-flight calls. The standalone
Win32 harness verifies stack/register conventions, content/stat invalidation, cursor movement, screen edges and command boundaries without game assets. Build it with Win32 MSVC
(`/EHsc /W4 /WX`, output `logs/tooltip_cache_harness.exe`), then run
`python scripts/test_tooltip_cache.py`. In that harness100identical draws require
one text build while all100render calls execute; hundreds of cursor movements
retain the text while item/stat changes rebuild immediately. This is not a measured game
FPS claim; native-client tooltip performance still needs owner verification.

## Basic melee final-position synchronization

The isolated profile enables `melee_sync` by default. A live Orc Watcher capture
showed repeated R refusals because a final1.28m player displacement was absent
from the last native stop packet; the target position/radii matched. R's client
surface-distance field cannot repair authoritative coordinates safely by itself.

At the exact native R send call00479CA0 (original call0069E550), compare the
player's current horizontal position (`[009DD3AC]+98/A0`) with GameProcMain's
last movement-send position(+28C/294). If it changed, call native MsgSend_Move
0069E5F0 first: moving state(+31C) uses(true,true) for the usual forecast, while
stopped state uses(false,false) for the actual position. Only a changed stop
clears native duplicate-stop suppression byte009DC960 before sending. Unchanged
positions do not produce additional movement. Then call the original R function
with its target, interval and distance arguments preserved. Native packet order
places the normal validated movement before attack; no new opcode, damage rule
or server range exemption is introduced. The server still rejects forged movement.

This hook has a separate `melee-position-<pid>.json` version1 receipt and the same
isolated-path/hash/signature checks and reversible process-memory installer.
Original client/reference files remain unchanged. Build
`scripts/tests/melee_sync_harness.cpp` with Win32 MSVC(`/EHsc /W4 /WX`, output
`logs/melee_sync_harness.exe`), then run `python scripts/test_tooltip_cache.py`.
The existing tooltip and autoloot harness still runs separately in that suite.

The server starter waits up to30seconds for its own two listeners before
launching the client-startup observer; slow cold asset loads must not skip the
observer because of the previous fixed two-second readiness check.
