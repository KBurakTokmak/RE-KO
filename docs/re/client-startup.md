# Original-client startup

The repeated idle/combat exit was traced to explicit self-termination, not an
unhandled game exception. A debugger caught TerminateProcess(self,0) at uptime
about356seconds, return address0078D93B. Watchdog0078D8C0 waits on event009E02F4
with120000ms timeouts; a missing heartbeat flag009E02F8 reaches a delayed exit.
Handler0078D990 supplies that flag for status0x5A004. This establishes the specific
timed-exit cause, not the cause of every future crash.

`scripts/Start-LocalClient.cmd` invokes the supported standalone startup profile.
`local_client.py` checks the exact executable SHA256 and unpacked watchdog
signature, duplicates the event handle and signals the watchdog's own shutdown
path. It verifies cleanup without modifying original files or writing process
memory. The server watcher can apply the same profile to an exact newly created
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
