# Contributor and agent guide

Read [README](README.md), the [documentation index](docs/README.md),
[reference data](docs/reference-data.md) and the relevant subsystem notes before
changing gameplay. The [backlog](docs/design/1453-feature-backlog.md) records
bounded coverage and missing systems; a handler's existence is not full fidelity.

## Scope and rules

- Target Knight Online 1453, singleplayer and small LAN sessions. Current work
  is the Rust standalone server with the original client; no replacement client
  or renderer exists yet. Keep MMO infrastructure outside the present scope.
- The server owns gameplay decisions. SQLite is the sole runtime persistence
  backend. External reference databases are read-only import/research inputs.
- Never commit original game files, binaries, assets, decrypted/decompiled
  dumps, reference packages, save databases, secrets, captures or build outputs.
- Never modify the original client installation or reference checkouts.
- Original 1453 client tables/native behavior and owner requirements take
  priority over supplied 1453 customizations, Open-KO 1298/9 and 2086 references.
  Consult the matching primary and Open-KO subsystem sources when available.
  Record mismatches/unknowns; do not silently invent mechanics or rewards.
- Preserve confirmed equipment, loot, potion, combat and quest behavior.
  Administrative grants must not become ordinary creation/startup defaults.
- Keep scripts and documentation path-independent. Resolve paths from the
  repository/script location or configuration; do not embed personal drive paths.
- Preserve required license attribution when incorporating substantial source.

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
