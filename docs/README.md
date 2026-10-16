# Documentation

This is the maintained documentation set. Session transcripts, capture files,
generated exports, account lists and deployment history do not belong here.

| Document | Use |
| --- | --- |
| [Setup](setup.md) | Required inputs, build, data import and start/stop commands |
| [Architecture](architecture.md) | Workspace, authority, persistence and current limits |
| [Reference data](reference-data.md) | Evidence order, local reference layout and reproducibility |
| [Testing](testing.md) | Automated checks and useful in-game regression tests |
| [Feature backlog](design/1453-feature-backlog.md) | Selectable missing/partial systems with source pointers |
| [Protocol](re/protocol.md) | Native packet layouts and UI ownership requirements |
| [Tables](re/tbl.md) | TBL cipher and binary layout |
| [Skills and combat](re/skills-combat.md) | Supported effects, formulas, costs and known gaps |
| [World and services](re/world-services.md) | Spawns, AI, loot, equipment, merchants and travel |
| [Quests and progression](re/quests-progression.md) | Concurrent objectives, exchange safety, XP and promotion |
| [Upgrade rates](re/upgrade-rates.md) | Table rates versus effective normal-upgrade probabilities |
| [Client startup](re/client-startup.md) | Timed-exit diagnosis and supported local launcher |

Record durable findings beside the relevant subsystem, with the source path or
native address and whether behavior is observed, inferred or deliberately chosen.
Update existing claims when behavior changes instead of appending contradictory
historical updates. Keep generated reports and raw/decompiled data outside Git.
