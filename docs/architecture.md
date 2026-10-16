# Architecture

The workspace implements an authoritative1453-compatible server in Rust, with
SQLite persistence. `reko-serverd` hosts `reko-server` over the original framed
TCP protocol. A future in-process host should reuse that library; a new client
and renderer are longer-term work, not existing crates.

| Crate | Responsibility |
| --- | --- |
| reko-common | Shared constants and installation path resolution |
| reko-assets | Pure table parsing and encryption/decryption |
| reko-server | Login/game handlers, sessions, simulation, gameplay and save DB |
| reko-serverd | Process host, logging and listener lifecycle |
| reko-tools | Table inspection, asset inventory and Windows helper executable |

The server validates ownership, range, resources, cooldowns, inventory capacity
and applicable source requirements. Persistence commits before success replies.
NPC scripts execute through a restricted bridge; they do not get filesystem,
network, OS or arbitrary database access. SQLite is the only runtime database.
The external SQL Server instances are read-only research/import sources.

Original1453 tables supply items, classes, skill definitions and XP thresholds.
Imported SQLite rules supply source-backed spawns, drops, AI, recipes, quests
and travel actions. Avoid hardcoded balance replacements. Known policy choices
include level cap80,300 starting NP,20,000 starting Noah and40 active quests.
Administrative fixture grants are explicitly separate from normal defaults.

Entity visibility uses48-metre regions and a3x3 neighborhood. Monster simulation
includes direct pursuit and bounded roaming; terrain pathfinding and full
event controllers are incomplete. There is no complete shared-player world,
party or PvP implementation. MAX_PLAYERS20 is a small-server design target,
not a demonstrated20-player compatibility/performance guarantee.

Login registers the first password and verifies its persisted Argon2id hash.
Accounts choose a permanent faction for the logical server before creating
characters; existing rosters retain their faction. See [accounts](re/accounts.md).
The game connection consumes a recent single-use login association by IP;
ambiguous/expired associations are rejected. Native token binding and simultaneous
clients behind one IP remain unfinished. The current executable must stay in
a trusted development environment.

Tests combine isolated SQLite/socket scenarios with optional original-asset
and reference-data checks. Passing them does not establish every native UI,
quest branch or skill effect. See [testing](testing.md) and the
[backlog](design/1453-feature-backlog.md).
