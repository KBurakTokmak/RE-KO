# Reference data and provenance

## Evidence order

1. Original1453 client tables, observed native behavior and explicit gameplay
   requirements from the project owner.
2. The supplied1453 server source/database, checking private customizations.
3. Open-KO1298/9 for corroboration and old Moradon map geometry.
4. Supplied2086 source/binaries/database for supplementary mechanics.

A matching field name or later-version implementation is not proof of1453
equivalence. Keep unknown behavior documented; do not invent rewards or copy
newer-map coordinates into old Moradon. Reference packages and the original
client remain read-only and outside this repository.

## Local layout currently used by scripts

| Sibling path relative to RE-KO | Purpose |
| --- | --- |
| `../KnightOnline1453` | Compatible original assets/client; Rust supports REKO_GAME_DIR |
| `../1453_New_2025/Source` | Primary GameServer/AIServer/shared source reference |
| `../PV_1453/inspection` | CSV exports of supplied1453 gameplay rule tables |
| `../KnightOnline` | Open-KO1298/9 checkout and historical server SMD maps |
| `../kn_2086_patch_private_server/lCAFILES/Quests` | Supplementary Lua content |

Open-KO upstream is `https://github.com/Open-KO/KnightOnline`, inspected revision
`7d6cf81093e142c928c2ac9510512b2b182178b5`. Consult its license before copying
substantial source. Reference pointers in the backlog name external files;
those files are not included by Git links or submodules.

The1453 import scripts read K_NPCPOS, K_NPC, K_MONSTER, ZONE_INFO, loot groups,
ITEM_UPGRADE, ITEM_EXCHANGE, QUEST_HELPER, QUEST_MONSTER, LEVEL_UP and related
rule exports. `Import-MapTriggers.py` additionally queries EVENT from the local
`.\TEW_SQLEXPRESS`/`pv1453_test` reference and reads historical SMD terrain tiles.
This machine-specific importer needs adapting for a different reference instance.
Running the supplied C++ server is outside the RE-KO setup path.

Legacy local inputs: `logs/reference-coefficients.csv` and optional
`logs/reference-rules/MAGIC_TYPE5.csv` originate from supplementary2086 rule
exports. Coefficients must match the CSV header expected by `combat.rs`;
private export files are not shipped. Optional broad audits also use local
import-fixture SQLite files and captures. Keep those files out of Git.

## Source navigation

| Subsystem | Primary1453 source | Open-KO corroboration |
| --- | --- | --- |
| NPC/monster population | AIServer/ServerDlg.cpp, Npc.cpp | AIServer/AIServerApp.cpp, Map.cpp |
| Damage, stats, regen | GameServer/User.cpp, Unit.cpp, MagicProcess.cpp | Ebenezer/User.cpp, MagicProcess.cpp |
| Shops/storage/upgrades | GameServer/NPCHandler.cpp, WarehouseSystem.cpp, UpgradeHandler.cpp | Ebenezer/User.cpp, client UITransactionDlg/UIItemUpgrade |
| Quest/events/travel | GameServer/QuestHandler.cpp, Map.cpp, ItemHandler.cpp | Ebenezer/GameEvent.cpp, EXEC.cpp, client GameProcMain.cpp |
| Loot | AIServer/Npc.cpp, GameServer/AISocket.cpp/User.cpp | AIServer/Npc.cpp, Ebenezer/User.cpp |

Analyze a separately obtained unpacked client image, not the protected disk EXE.
Use read-only Ghidra/Capstone investigation; store dumps, decompiled output,
analysis projects and traces locally. Record useful addresses and layouts in
the relevant maintained protocol/subsystem document instead of committing dumps.
