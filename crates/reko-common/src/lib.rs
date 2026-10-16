//! Shared types: game paths, packet defs, table schemas.

use std::path::PathBuf;

/// Env var pointing at the original KO install.
pub const GAME_DIR_ENV: &str = "REKO_GAME_DIR";

/// Original install dir. Env override, else sibling `KnightOnline1453`.
pub fn game_dir() -> PathBuf {
    std::env::var_os(GAME_DIR_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("../KnightOnline1453"))
}
