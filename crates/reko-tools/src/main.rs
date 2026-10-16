//! Dev CLI: asset inspection, tbl dump, packet tools.

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "reko")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Asset file counts per extension in the game dir.
    Inventory,

    /// Decrypt and print rows from a .tbl file.
    DumpTbl {
        /// Path to .tbl file (relative to game dir or absolute).
        path: std::path::PathBuf,
        /// Maximum rows to display.
        #[arg(short, long, default_value_t = 10)]
        limit: usize,
        /// Display every column instead of the first eight.
        #[arg(long)]
        full: bool,
    },
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().cmd {
        Cmd::Inventory => inventory(),
        Cmd::DumpTbl { path, limit, full } => dump_tbl(path, limit, full),
    }
}

fn inventory() -> anyhow::Result<()> {
    use std::collections::BTreeMap;
    let root = reko_common::game_dir();
    let mut counts: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir)? {
            let e = e?;
            let meta = e.metadata()?;
            if meta.is_dir() {
                stack.push(e.path());
                continue;
            }
            let ext = e
                .path()
                .extension()
                .map(|x| x.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            let c = counts.entry(ext).or_default();
            c.0 += 1;
            c.1 += meta.len();
        }
    }
    println!("{}", root.display());
    for (ext, (n, bytes)) in counts {
        println!(
            "{ext:>10} {n:>6} files {:>9.1} MB",
            bytes as f64 / 1_048_576.0
        );
    }
    Ok(())
}

fn dump_tbl(path: std::path::PathBuf, limit: usize, full: bool) -> anyhow::Result<()> {
    let resolved = if path.is_absolute() || path.exists() {
        path
    } else {
        reko_common::game_dir().join(path)
    };

    let raw = std::fs::read(&resolved)?;
    let table = reko_assets::Table::from_encrypted_bytes(&raw)?;
    println!(
        "{}: {} columns, {} rows",
        resolved.display(),
        table.columns.len(),
        table.rows.len()
    );

    for (idx, row) in table.rows.iter().take(limit).enumerate() {
        print!("  [{idx:>3}] ");
        let columns = if full { row.len() } else { 8 };
        for (c_idx, cell) in row.iter().take(columns).enumerate() {
            if c_idx > 0 {
                print!(", ");
            }
            match cell {
                reko_assets::CellValue::String(s) => print!("{s:?}"),
                reko_assets::CellValue::I8(v) => print!("{v}"),
                reko_assets::CellValue::U8(v) => print!("{v}"),
                reko_assets::CellValue::I16(v) => print!("{v}"),
                reko_assets::CellValue::U16(v) => print!("{v}"),
                reko_assets::CellValue::I32(v) => print!("{v}"),
                reko_assets::CellValue::U32(v) => print!("{v}"),
                reko_assets::CellValue::F32(v) => print!("{v:.2}"),
                reko_assets::CellValue::F64(v) => print!("{v:.2}"),
                reko_assets::CellValue::I64(v) => print!("{v}"),
                reko_assets::CellValue::U64(v) => print!("{v}"),
            }
        }
        if !full && row.len() > 8 {
            print!(", ... ({} more columns)", row.len() - 8);
        }
        println!();
    }

    Ok(())
}
