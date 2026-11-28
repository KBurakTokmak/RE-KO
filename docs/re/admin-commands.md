# Chat commands

Type these in **All** chat without a leading slash; the original client can
swallow unknown slash commands. Admin commands need the character's database
administrator role, which is rechecked on every command. For other characters
the same text is sent as ordinary chat. Details and evidence are in
[admin testing](admin-testing.md).

## Admin commands

| Command | Usage | Description |
| --- | --- | --- |
| `op players` | `op players [page]` | Private online roster, five names per page, with zone |
| `op kick` | `op kick <name>` | Disconnect a character; the account can sign in again |
| `op announce` | `op announce <text>` | Server notice to every player in all zones |
| `op mute` | `op mute <name> <1-60>` | Mute the character's account for N minutes |
| `op unmute` | `op unmute <name>` | Remove a mute |
| `op invade` | `op invade <zone> on\|off` | Make a zone's guards and towers attack enemy-nation players |
| `op weather` | `op weather clear\|rain\|snow <0-100>` | Set world weather until the next hour |
| `op event list` | `op event list` | Configured event schedule rows |
| `op event status` | `op event status` | Running events: phase, stage, scores, owner |
| `op event start` | `op event start temple [level]\|lunar [61\|62\|64]\|darklunar\|siege` | Start an event now; the temple level picks the bracket |
| `op event stop` | `op event stop <event>` | Stop at once: despawn, no rewards, players leave |
| `op event advance` | `op event advance <event>` | Skip temple sign-up or to the next wave, war to the half-time result, siege countdown |
| `op event result` | `op event result karus\|elmorad\|draw` | Decide the running war |
| `gmvisible` | `gmvisible on\|off\|status` | Show or hide this admin to normal players (session only) |
| `teleport`, `tp` | `tp <x> <z>` | Warp within the current zone to map coordinates |
| `zonechange` | `zonechange [id\|name]` | Move to a zone; no argument lists zones |
| `spawn`, `+monsummon` | `spawn <id\|name>` | Summon one temporary monster next to you |
| `mobfind` | `mobfind [page] <name>` | Search monster prototypes by name |
| `bossstatus` | `bossstatus [name]` | Natural rare boss positions or respawn timers in this zone |
| `hurt` | `hurt <amount>` | Damage yourself for testing (stops at 1 HP) |

Admins also have fixed 30,000-damage basic attacks, monster protection and GM
collision bypass; see [admin testing](admin-testing.md).

## Player commands

| Command | Usage | Description |
| --- | --- | --- |
| `autoloot` | `autoloot on\|off` | Automatic loot pickup within range (off at login) |
| `quests` | `quests` | List active quests |
| `quest` | `quest <id>` | Show one quest's description and counters |
| `questcancel` | `questcancel <id>` | Abandon an active quest |
| `/town` | `/town` | Return to the zone's town start |
