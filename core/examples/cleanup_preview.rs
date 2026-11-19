//! Read-only cleanup preview: `cargo run -p omsi-tools-core --example cleanup_preview -- <omsi2> [neoomsi] [openomsi]`
//! Lists the unused Sceneryobjects / Splines / Vehicles folders. Nothing is changed.
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use omsi_tools_core::cleanup::analyze;
use omsi_tools_core::maps::list_maps;
use omsi_tools_core::Config;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = |i: usize| args.get(i).filter(|s| !s.is_empty() && s.as_str() != "-").map(PathBuf::from);
    let cfg = Config { omsi_dir: dir(0), neo_omsi_dir: dir(1), open_omsi_dir: dir(2), ..Default::default() };
    let dirs = cfg.game_dirs();
    let maps = list_maps(&dirs);
    let r = analyze(&dirs, &maps, &AtomicBool::new(false), &mut |_| {});
    println!("{} maps checked, {} unused folders", r.maps_scanned, r.candidates.len());
    for c in &r.candidates {
        println!("  [{}] {}  ({} files, {} bytes)", c.source, c.relative, c.file_count, c.size_bytes);
    }
}
