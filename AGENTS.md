# Contributor guide

RE-KO is a Rust server for the original 1453 client, for singleplayer and small LAN sessions. Start with the [README](README.md), [reference guide](docs/reference-data.md) and the [subsystem notes](docs/README.md). Use the [backlog](docs/design/1453-feature-backlog.md) to find gaps and confirm them in the current code.

## Before changing gameplay

- Fetch and compare Git history, preserve local work, and verify the running server and available test inputs. Run a [baseline](docs/testing.md). Local handovers can be stale.
- After pulling, run `python scripts/database_manager.py check`. Apply outstanding migrations with the server stopped and the save backed up. Use the [setup guide](docs/setup.md) for fresh databases and missing inputs.
- Follow owner requirements and original 1453 data and native behavior first. Compare the supplied 1453 server and Open-KO, using 2086 as supplementary evidence. Record meaningful version differences and unresolved questions beside the subsystem.
- Keep original client installations and reference packages read-only. Obtain explicit owner approval for any new isolated-client file or process-memory change. Keep existing server-only balance decisions and approved client remedies.

## Implementation

- The server owns gameplay and SQLite owns runtime persistence. Validate ownership, costs, scope and repeated requests. Commit transactions before reporting success. Keep administrative grants separate from ordinary defaults.
- Register persistent data changes with the database manager, keep schema versions aligned, and test fresh creation, upgrades, rollback and repeat runs. Preserve local saves and customizations.
- Keep gameplay portable and paths configurable. Isolate platform-specific unsafe code and explain its safety. Preserve source attribution.
- Keep maintained docs concise. Update existing subsystem notes and remove obsolete reports and broken references. Keep private assets, binaries, databases, captures, credentials, generated research and test output outside Git.

## Verification and review

- Run `cargo fmt --all -- --check`, `cargo test --workspace` with available original assets, and `cargo clippy --all-targets -- -D warnings`. Follow [testing](docs/testing.md) for database checks and native-client coverage.
- After implementation, have a separate subagent review the complete change against the request and reference behavior. Ask it to check correctness, regressions and test gaps. Fix confirmed findings, rerun affected checks, and have the reviewer verify substantive fixes before finishing.
- Report validation and remaining in-game checks. Push only with explicit owner confirmation and open pull requests only when requested.
