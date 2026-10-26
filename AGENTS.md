# Contributor and agent guide

Read [README](README.md), the [documentation index](docs/README.md),
[reference data](docs/reference-data.md) and the relevant subsystem notes before
changing gameplay. The [backlog](docs/design/1453-feature-backlog.md) records
bounded coverage and missing systems; a handler's existence is not full fidelity.

## Resuming work

- Compare the checkout and fetched upstream history before continuing an older
  chat. Use current subsystem docs and code to establish coverage; do not treat
  an old handover's commit, process ID, test count or missing-feature list as
  current state. Preserve local changes when updating the checkout.
- Review [testing](docs/testing.md) and run a baseline with the available local
  inputs before selecting the next implementation. Distinguish code regressions
  from missing imports, optional fixtures and native-client checks not exercised.
- Recent systems already include clan promotion/capes, friendly and party skill
  targeting, Mage special effects, transformations/PUS items, costumes, NPC
  exchanges, combat-zone travel, GM visibility and multi-arrow flight tracking.
  Consult [social](docs/re/social.md), [skills/combat](docs/re/skills-combat.md),
  [world services](docs/re/world-services.md) and
  [admin testing](docs/re/admin-testing.md) before reimplementing those paths.

## Scope and rules

- Target Knight Online 1453, singleplayer and small LAN sessions. Current work
  is the Rust standalone server with the original client; no replacement client
  or renderer exists yet. Keep MMO infrastructure outside the present scope.
- The server owns gameplay decisions. SQLite is the sole runtime persistence
  backend. External reference databases are read-only import/research inputs.
- Never commit original game files, binaries, assets, decrypted/decompiled
  dumps, reference packages, save databases, secrets, captures or build outputs.
- Never modify the original client installation or reference checkouts.
- Keep existing approved local client remedies. Before any new client file or
  process-memory change, explain the need and affected files/areas to the owner
  and wait for explicit approval. Prefer server-side fixes where feasible.
- Original 1453 client tables/native behavior and owner requirements take
  priority over supplied 1453 customizations, Open-KO 1298/9 and 2086 references.
  Consult the matching primary and Open-KO subsystem sources when available.
  Record mismatches/unknowns; do not silently invent mechanics or rewards.
- Preserve confirmed equipment, loot, potion, combat and quest behavior.
  Administrative grants must not become ordinary creation/startup defaults.
- Keep scripts and documentation path-independent. Resolve paths from the
  repository/script location or configuration; do not embed personal drive paths.
- Preserve required license attribution when incorporating substantial source.
- Wait for explicit owner confirmation before pushing to GitHub. Open pull
  requests only when requested; do not infer push approval from implementation work.

## Implementation

Rust stable/edition 2024, Windows 11 x64/MSVC. Keep gameplay and asset readers
portable; isolate Windows launcher/FFI work. No unsafe outside platform/FFI
modules without a `SAFETY:` explanation. Use original tables/imported rules
instead of hardcoded balance data. Commit persistence before success packets;
validate ownership, scope, replay, resources and rollback as appropriate.

Keep code comments short and local. Put reverse-engineering provenance, native
addresses, confidence and version differences in the subsystem docs. Update
existing documentation when behavior changes; do not append session histories,
process IDs, account details or raw log extracts.

## Documentation and repository hygiene

- Keep documentation small, relevant and maintained. Prefer updating an existing
  subsystem document over creating a separate report for each task or fix.
- After implementing and verifying a feature, review its supporting documents.
  Remove unrelated, duplicated, superseded or temporary documents, test-result
  reports and research write-ups when they have no remaining maintenance,
  reproducibility or future implementation use. Fix affected links and indexes.
- Fold still-useful findings into concise subsystem notes before removing a
  redundant report. Preserve essential protocol/source evidence, unresolved
  questions, setup instructions and required license attribution.
- Keep raw test outputs, captures, generated research and intermediate analysis
  outside tracked repository content. Do not add historical pass counts or
  session-by-session results to maintained docs; report them in the task/PR.
- Retain useful automated regression tests and required reference inputs. This
  cleanup rule concerns obsolete reports and artifacts, not deleting coverage
  merely because the feature is implemented. Do not move unnecessary documents
  into a tracked archive just to avoid removing them.

Before finishing implementation changes:

```powershell
cargo fmt --all
cargo test --workspace
cargo clippy --all-targets -- -D warnings
```

Set REKO_GAME_DIR for original-asset tests and provision optional reference
inputs for integration coverage. Report skipped/unavailable coverage accurately.
Do not add implementation-mirroring tests for simple documentation changes.
Use meaningful socket/SQLite/behavioral regressions for gameplay changes.

Run from the workspace root. See [setup](docs/setup.md) for data imports and
server start/stop. Stop the Windows daemon before replacing its executable or
offline imports; back up the save and verify unrelated data is preserved.
Never replay old one-off grants or edit a live character merely to satisfy tests.

## Runtime and regression safeguards

- When a player is connected before a planned shutdown/restart, have them exit
  or log out through the native menu while the server is still running. Check
  for the accepted opcode79/1 skillbar save when shortcut persistence matters.
  SQLite can preserve
  only received layouts; never invent an autosave request, reconstruct bars from
  skill use or reset them to defaults. The owner declined a new client change
  for unsent skillbar edits; see [skillbar](docs/re/skillbar.md).
- The start BAT pulls with `git pull --ff-only` before startup; the stop BAT
  attempts the same after stopping. Direct PowerShell launchers do not pull.
  Account for that difference when inspecting changes or selecting a build.
- Reference imports/repairs and local client preparation are separate from a
  Git update. Inspect their required exports, existing save rules and dry runs
  before applying them; do not assume pulling commits provisions gameplay data.
- After pulling, check database compatibility with `python scripts/database_manager.py check`.
  Apply required schema/content migrations with the server stopped before testing;
  the standard startup script runs `update` automatically. Preserve each installation's
  accounts, characters, clans, inventory and local customizations. Never replace an
  existing save with another developer's database or silently factory-reset it.
- Register new persistent world-data fixes in the ordered content migrations in
  `scripts/database_manager.py`; bump the schema version and add a data-preserving
  migration when changing schema. Keep the Rust schema version and manager in sync.
  Test upgrades, repeat runs, rollback and fresh creation from a world-only template.
  Templates/reference exports stay ignored and separate from Git; update the setup
  instructions when a new input is required. Missing inputs must fail clearly.
- Reconcile every outstanding feature import with the migration registry when
  adopting changes from another checkout. Do not declare a database current
  after applying only the feature reported by the player. Initial world imports
  and one-off administrative grants are not repeatable catch-up migrations.
- Preserve the approved isolated-client remedies described in
  [client startup](docs/re/client-startup.md). Read-only analysis and standalone
  harness tests do not authorize new client-file or process-memory patches.
- Player visibility is viewer-specific. Follow `social.rs` and the admin guide
  when testing GM self/party/normal-player views; do not globally expose hidden
  GMs merely to repair a roster or a friendly-target failure.
- Archery uses paid per-flight indices and bounded impact quotas, including the
  native duplicated base index. Preserve launch-position validation, cross-skill
  combos and harmless late cleanup; do not add an invented shared recovery lock
  or make every arrow hit the selected target. See the combat subsystem notes.
