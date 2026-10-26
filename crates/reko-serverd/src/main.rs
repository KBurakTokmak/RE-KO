//! Standalone LAN server.

use std::fs::{self, OpenOptions};
use std::path::PathBuf;
use std::sync::Mutex;
use tracing_subscriber::prelude::*;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut arguments = std::env::args_os().skip(1);
    if let Some(command) = arguments.next() {
        anyhow::ensure!(
            command == "--prepare-db",
            "usage: reko-serverd [--prepare-db PATH]"
        );
        let path = arguments
            .next()
            .ok_or_else(|| anyhow::anyhow!("missing database path"))?;
        anyhow::ensure!(arguments.next().is_none(), "unexpected database arguments");
        let cfg = reko_server::Config {
            db_path: path.into(),
            ..Default::default()
        };
        let db = reko_server::open_db(&cfg)?;
        reko_server::database::initialize(&db)?;
        db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
        println!("Database schema ready: {}", cfg.db_path.display());
        return Ok(());
    }
    let log_path = std::env::var_os("REKO_LOG_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("logs/server.log"));
    if let Some(parent) = log_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(Mutex::new(log)),
        )
        .init();
    tracing::info!(pid = std::process::id(), log = %log_path.display(), "server watcher started");
    tokio::select! {
        result = reko_server::run(reko_server::Config::default()) => {
            if let Err(ref error) = result {
                tracing::error!(%error, "server stopped with error");
            }
            result
        }
        result = tokio::signal::ctrl_c() => {
            result?;
            tracing::info!("server stopped by Ctrl+C");
            Ok(())
        }
    }
}
