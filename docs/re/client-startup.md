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
Other builds are rejected. The helper requires Windows, Python3, the expected
sibling client layout and local Server.ini configuration. Existing game instances
must be closed before using the explicit launcher. No permanent XTrap stripping
or EXE dependency removal has been performed. Startup debugger attachment and
bootstrap-skipping experiments were unsuccessful and are not supported launchers.

Sessions beyond the former exit window were validated, including a21-minute
idle run closed intentionally by the tester. A later run exceeded an hour.
Future exits still need their own process/packet evidence. Keep raw dumps,
debugger traces, protected binaries and generated decompilation outside Git.
