//! Read-only command line scan: `cargo run -p omsi-tools-core --example scan -- <omsi2> [neoomsi] [openomsi] [map name filter]`
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use omsi_tools_core::maps::list_maps;
use omsi_tools_core::scanner::{scan_map, FileKind};
use omsi_tools_core::{Config, Source};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = |i: usize| args.get(i).filter(|s| !s.is_empty() && s.as_str() != "-").map(PathBuf::from);
    let cfg = Config { omsi_dir: dir(0), neo_omsi_dir: dir(1), open_omsi_dir: dir(2), cleanup_dir: None };
    let dirs = cfg.game_dirs();
    let filter = args.get(3).map(|s| s.to_lowercase());
    let maps = list_maps(&dirs);
    println!("{} maps in {} directories", maps.len(), dirs.len());
    for (_, d) in dirs.iter() {
        println!("  dir: {}", d.display());
    }
    for m in maps.iter().filter(|m| filter.as_ref().map_or(false, |f| m.name.to_lowercase().contains(f))) {
        let r = scan_map(&dirs, m, &AtomicBool::new(false), &mut |_| {});
        println!("\n== {} ({}): {} missing of {} files", m.label(), m.source, r.missing_count(), r.entries.len());
        for k in FileKind::ALL {
            for e in r.missing_of(k) {
                println!("  MISSING {:?} {}   <- {}", k, e.path, e.referenced_by.join(", "));
            }
        }
        let _ = Source::Omsi2;
    }
    if filter.is_none() {
        for m in &maps {
            println!("  {}", m.label());
        }
    }
}
