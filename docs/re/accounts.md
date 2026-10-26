# Accounts, passwords and server faction

Login F3 parses the complete length-prefixed username/password packet and rejects
truncated, trailing, empty or invalid input before registration. Account names
are case-insensitive ASCII letters/digits/underscore, up to 32 bytes; passwords
are case-sensitive, 1..28 bytes with no embedded NUL. The first accepted login
registers a password. Later attempts must verify it; wrong credentials receive
native failure 3.

`account_passwords` stores a salted Argon2id PHC hash, never the plaintext
password. Random salts and verification use the
[RustCrypto Argon2 API](https://docs.rs/argon2/0.5.3/argon2/).
Hash work runs off the async executor with four concurrent jobs, outside the
gameplay database lock. Registration inserts transactionally without replacing
another first login's password; a race verifies the winning registration.

Existing accounts had no password to migrate. Their first successful login
after this update sets one, preserving characters and their existing faction.
There is no default/reset password and no automatic password overwrite.
Login and game credential packets are excluded from hex captures. Username
association is issued only after successful verification, and a failed/new
login attempt clears stale pending association for that IP.

## Nation selection

`account_server_nations` keys the choice by account/server ID, with Karus 1 or
El Morad 2. The currently supported logical server is 1; the project still has
one advertised server. No membership means unselected 0: game login replies
`01 00`, which opens the native nation-choice screen. Selection 05 persists
the first choice; same-choice retries succeed, opposite-choice retries fail.
Character creation rejects an unselected or mismatched race/faction. The choice
remains fixed even if the roster becomes empty.

Startup preserves existing characters' faction by adding only missing server
memberships inferred from their race. Empty legacy accounts must choose instead
of inheriting the old default Karus value. Completed choices are never reset.
The legacy lobby_accounts.nation field is kept synchronized for offline tools;
the new membership is authoritative for the client flow.

Source evidence: supplied 1453 LogInServer/LoginSession.cpp defines login
success 1/invalid 3 and the 28-byte password limit; GameServer/DatabaseThread.cpp
ReqAccountLogIn/ReqSelectNation return nation 0/1/2 or failure FF. Open-KO
VersionManager/User.cpp LogInReq and client GameProcLogIn_1298.cpp
MsgRecv_GameServerLogIn corroborate this. First-login registration is the
owner's requested policy, not copied SQL/ODBC account provisioning.

The game handoff still consumes a single-use 60-second IP association; ambiguous
same-IP sessions are refused. Recovering native token binding, duplicate live
session handling, password recovery and stronger transport are separate work.
Passwords now validate, but the legacy protocol is not modern secure internet
transport. Keep the server on trusted development/LAN networks.
The server-list handoff automatically uses the local IPv4 of each accepted login
socket. Localhost/LAN/VPN connections therefore receive the address they connected
to, preserving the current IP-based login/game association. `REKO_ADVERTISE_IP` or
starter `-AdvertiseIp` optionally forces a host address for NAT/proxy setups; there
is no interface scan or startup prompt. See [setup](../setup.md).
