// OMSI-Tools-NG-rs — derived from OMSI-Tools by pietr26 (GPLv2). See LICENSE / README.
#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use omsi_tools_core::cleanup::{self, Action, Candidate, CleanupReport};
use omsi_tools_core::maps::{list_maps, MapEntry};
use omsi_tools_core::scanner::{scan_map, FileKind, Progress, ScanReport};
use omsi_tools_core::Config;
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel, Weak};

slint::include_modules!();

#[derive(Clone, PartialEq)]
enum Pending {
    None,
    Move(PathBuf),
    Recycle,
}

struct State {
    cfg: Config,
    cfg_path: Option<PathBuf>,
    maps: Vec<MapEntry>,
    report: Option<ScanReport>,
    cleanup: Option<CleanupReport>,
    checked: Vec<bool>,
    pending: Pending,
    cancel: Arc<AtomicBool>,
}

type Shared = Arc<Mutex<State>>;

fn human_size(b: u64) -> String {
    const U: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 { format!("{b} B") } else { format!("{v:.1} {}", U[i]) }
}

fn path_str(p: &Option<PathBuf>) -> SharedString {
    p.as_ref().map(|p| p.display().to_string()).unwrap_or_default().into()
}

fn opt_path(s: &str) -> Option<PathBuf> {
    let s = s.trim();
    (!s.is_empty()).then(|| PathBuf::from(s))
}

fn save_cfg(st: &State) -> Option<String> {
    let path = st.cfg_path.as_ref()?;
    st.cfg.save(path).err().map(|e| format!("Could not save settings: {e}"))
}

fn model<T: Clone + 'static>(v: Vec<T>) -> ModelRc<T> {
    ModelRc::from(Rc::new(VecModel::from(v)))
}

fn reload_maps(ui: &MainWindow, st: &mut State) {
    st.maps = list_maps(&st.cfg.game_dirs());
    let items: Vec<MapItem> = st
        .maps
        .iter()
        .map(|m| MapItem {
            label: m.name.clone().into(),
            source: format!("{} · {}", m.source, m.dir.file_name().unwrap_or_default().to_string_lossy()).into(),
        })
        .collect();
    ui.set_maps(model(items));
    ui.set_selected_map(-1);
}

fn result_rows(r: &ScanReport, show_found: bool) -> Vec<ResultRow> {
    let mut rows = Vec::new();
    for kind in FileKind::ALL {
        let total = r.total_of(kind);
        if total == 0 {
            continue;
        }
        let missing = r.missing_of(kind).count();
        rows.push(ResultRow {
            header: true,
            missing: missing > 0,
            text: format!("{} — {} missing of {}", kind.label(), missing, total).into(),
            detail: SharedString::new(),
        });
        for e in r.entries.iter().filter(|e| e.kind == kind) {
            if !e.is_missing() && !show_found {
                continue;
            }
            let by = format!("referenced by: {}", e.referenced_by.join(", "));
            let detail = match &e.found {
                Some((src, _)) => format!("found in {src} · {by}"),
                None => by,
            };
            rows.push(ResultRow {
                header: false,
                missing: e.is_missing(),
                text: e.path.clone().into(),
                detail: detail.into(),
            });
        }
    }
    if !r.warnings.is_empty() {
        rows.push(ResultRow { header: true, missing: false, text: format!("Warnings ({})", r.warnings.len()).into(), detail: SharedString::new() });
        for w in &r.warnings {
            rows.push(ResultRow { header: false, missing: false, text: w.clone().into(), detail: SharedString::new() });
        }
    }
    rows
}

fn summary_text(r: &ScanReport) -> String {
    let missing = r.missing_count();
    let verdict = if missing == 0 { "no missing files".to_string() } else { format!("{missing} missing file(s)") };
    let cancelled = if r.cancelled { " (scan cancelled, partial result)" } else { "" };
    format!("{} [{}]: {} among {} referenced files{cancelled}", r.map_name, r.source, verdict, r.entries.len())
}

fn clean_rows(r: &CleanupReport, checked: &[bool]) -> Vec<CleanRow> {
    r.candidates
        .iter()
        .zip(checked)
        .map(|(c, &ch)| CleanRow {
            text: format!("{}  [{}]", c.relative, c.source).into(),
            detail: format!("{} files · {} · {}", c.file_count, human_size(c.size_bytes), c.path.display()).into(),
            checked: ch,
        })
        .collect()
}

fn update_selected(ui: &MainWindow, st: &State) {
    let Some(r) = &st.cleanup else {
        ui.set_clean_selected("".into());
        return;
    };
    let sel: Vec<&Candidate> = r.candidates.iter().zip(&st.checked).filter(|(_, c)| **c).map(|(c, _)| c).collect();
    let size: u64 = sel.iter().map(|c| c.size_bytes).sum();
    ui.set_clean_selected(format!("{} folder(s) selected · {}", sel.len(), human_size(size)).into());
}

fn on_ui<F: FnOnce(&MainWindow) + Send + 'static>(w: &Weak<MainWindow>, f: F) {
    let _ = w.upgrade_in_event_loop(move |ui| f(&ui));
}

fn progress_cb(w: Weak<MainWindow>) -> impl FnMut(Progress) {
    move |p: Progress| {
        let frac = if p.total == 0 { 0.0 } else { p.done as f32 / p.total as f32 };
        on_ui(&w, move |ui| {
            ui.set_progress(frac);
            ui.set_status(p.message.into());
        });
    }
}

fn pick_folder() -> Option<PathBuf> {
    rfd::FileDialog::new().pick_folder()
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = MainWindow::new()?;

    let cfg_path = Config::default_path();
    let (cfg, load_err) = match cfg_path.as_deref().map(Config::load) {
        Some(Ok(c)) => (c, None),
        Some(Err(e)) => (Config::default(), Some(format!("Could not load settings: {e}"))),
        None => (Config::default(), None),
    };
    let state: Shared = Arc::new(Mutex::new(State {
        cfg,
        cfg_path,
        maps: vec![],
        report: None,
        cleanup: None,
        checked: vec![],
        pending: Pending::None,
        cancel: Arc::new(AtomicBool::new(false)),
    }));

    {
        let mut st = state.lock().unwrap();
        ui.set_omsi_dir(path_str(&st.cfg.omsi_dir));
        ui.set_neo_dir(path_str(&st.cfg.neo_omsi_dir));
        ui.set_open_dir(path_str(&st.cfg.open_omsi_dir));
        ui.set_cleanup_dir(path_str(&st.cfg.cleanup_dir));
        ui.set_config_path(st.cfg_path.as_deref().map(Path::display).map(|d| d.to_string()).unwrap_or_default().into());
        ui.set_progress(0.0);
        ui.set_status(load_err.unwrap_or_default().into());
        if st.cfg.game_dirs().is_empty() {
            ui.set_page(2);
            ui.set_status("Select your OMSI 2 directory to get started.".into());
        }
        reload_maps(&ui, &mut st);
    }

    // --- settings ---
    let apply_dir = {
        let state = state.clone();
        let w = ui.as_weak();
        move |which: i32, value: Option<PathBuf>| {
            let ui = w.unwrap();
            let mut st = state.lock().unwrap();
            let (slot, set): (&mut Option<PathBuf>, fn(&MainWindow, SharedString)) = match which {
                0 => (&mut st.cfg.omsi_dir, MainWindow::set_omsi_dir),
                1 => (&mut st.cfg.neo_omsi_dir, MainWindow::set_neo_dir),
                2 => (&mut st.cfg.open_omsi_dir, MainWindow::set_open_dir),
                _ => (&mut st.cfg.cleanup_dir, MainWindow::set_cleanup_dir),
            };
            *slot = value;
            set(&ui, path_str(slot));
            let err = save_cfg(&st);
            reload_maps(&ui, &mut st);
            st.report = None;
            st.cleanup = None;
            ui.set_results(model(vec![]));
            ui.set_summary("".into());
            ui.set_clean_rows(model(vec![]));
            ui.set_clean_summary("".into());
            ui.set_status(err.unwrap_or_else(|| "Settings saved.".into()).into());
        }
    };
    {
        let apply = apply_dir.clone();
        ui.on_browse_dir(move |which| {
            if let Some(p) = pick_folder() {
                apply(which, Some(p));
            }
        });
    }
    ui.on_dir_changed(move |which, text| apply_dir(which, opt_path(&text)));

    {
        let state = state.clone();
        let w = ui.as_weak();
        ui.on_reload_maps(move || {
            let ui = w.unwrap();
            reload_maps(&ui, &mut state.lock().unwrap());
        });
    }

    // --- scan ---
    {
        let state = state.clone();
        let w = ui.as_weak();
        ui.on_scan(move |idx| {
            let ui = w.unwrap();
            let (dirs, map, cancel) = {
                let st = state.lock().unwrap();
                let Some(map) = usize::try_from(idx).ok().and_then(|i| st.maps.get(i)).cloned() else { return };
                st.cancel.store(false, Ordering::Relaxed);
                (st.cfg.game_dirs(), map, st.cancel.clone())
            };
            ui.set_busy(true);
            ui.set_progress(0.0);
            ui.set_summary("".into());
            ui.set_results(model(vec![]));
            ui.set_status(format!("Scanning {} …", map.label()).into());

            let state = state.clone();
            let w = w.clone();
            std::thread::spawn(move || {
                let mut cb = progress_cb(w.clone());
                let report = scan_map(&dirs, &map, &cancel, &mut cb);
                state.lock().unwrap().report = Some(report.clone());
                on_ui(&w, move |ui| {
                    let rows = result_rows(&report, ui.get_show_found());
                    ui.set_results(model(rows));
                    ui.set_summary(summary_text(&report).into());
                    ui.set_progress(1.0);
                    ui.set_status(if report.cancelled { "Scan cancelled." } else { "Scan finished." }.into());
                    ui.set_busy(false);
                });
            });
        });
    }
    {
        let state = state.clone();
        let w = ui.as_weak();
        ui.on_show_found_changed(move |show| {
            let ui = w.unwrap();
            if let Some(r) = &state.lock().unwrap().report {
                ui.set_results(model(result_rows(r, show)));
            }
        });
    }
    {
        let state = state.clone();
        ui.on_cancel(move || state.lock().unwrap().cancel.store(true, Ordering::Relaxed));
    }

    // --- cleanup ---
    {
        let state = state.clone();
        let w = ui.as_weak();
        ui.on_analyze(move || {
            let ui = w.unwrap();
            let (dirs, maps, cancel) = {
                let mut st = state.lock().unwrap();
                st.cancel.store(false, Ordering::Relaxed);
                st.pending = Pending::None;
                (st.cfg.game_dirs(), list_maps(&st.cfg.game_dirs()), st.cancel.clone())
            };
            if dirs.is_empty() {
                ui.set_status("Configure at least the OMSI 2 directory first.".into());
                return;
            }
            ui.set_busy(true);
            ui.set_pending_action("".into());
            ui.set_progress(0.0);
            ui.set_clean_summary("".into());
            ui.set_clean_rows(model(vec![]));
            let state = state.clone();
            let w = w.clone();
            std::thread::spawn(move || {
                let mut cb = progress_cb(w.clone());
                let report = cleanup::analyze(&dirs, &maps, &cancel, &mut cb);
                let checked = vec![false; report.candidates.len()];
                {
                    let mut st = state.lock().unwrap();
                    st.cleanup = Some(report.clone());
                    st.checked = checked.clone();
                }
                let state = state.clone();
                on_ui(&w, move |ui| {
                    ui.set_clean_rows(model(clean_rows(&report, &checked)));
                    let total: u64 = report.candidates.iter().map(|c| c.size_bytes).sum();
                    ui.set_clean_summary(
                        if report.cancelled {
                            "Analysis cancelled.".to_string()
                        } else {
                            format!(
                                "{} unused folder(s), {} in total — {} maps checked. Nothing has been changed.",
                                report.candidates.len(),
                                human_size(total),
                                report.maps_scanned
                            )
                        }
                        .into(),
                    );
                    ui.set_progress(1.0);
                    ui.set_status(if report.warnings.is_empty() { "Analysis finished.".into() } else { format!("Analysis finished with {} warning(s).", report.warnings.len()) }.into());
                    ui.set_busy(false);
                    update_selected(ui, &state.lock().unwrap());
                });
            });
        });
    }
    {
        let state = state.clone();
        let w = ui.as_weak();
        ui.on_clean_toggled(move |i, on| {
            let ui = w.unwrap();
            let mut st = state.lock().unwrap();
            if let Some(c) = usize::try_from(i).ok().and_then(|i| st.checked.get_mut(i)) {
                *c = on;
            }
            let rows = ui.get_clean_rows();
            if let Some(mut row) = rows.row_data(i as usize) {
                row.checked = on;
                rows.set_row_data(i as usize, row);
            }
            update_selected(&ui, &st);
        });
    }
    {
        let state = state.clone();
        let w = ui.as_weak();
        ui.on_clean_select_all(move |on| {
            let ui = w.unwrap();
            let mut st = state.lock().unwrap();
            st.checked.iter_mut().for_each(|c| *c = on);
            if let Some(r) = &st.cleanup {
                ui.set_clean_rows(model(clean_rows(r, &st.checked)));
            }
            update_selected(&ui, &st);
        });
    }
    let request = {
        let state = state.clone();
        let w = ui.as_weak();
        move |pending: Pending| {
            let ui = w.unwrap();
            let mut st = state.lock().unwrap();
            let n = st.checked.iter().filter(|c| **c).count();
            if n == 0 {
                ui.set_status("Select at least one folder first.".into());
                return;
            }
            let text = match &pending {
                Pending::Move(d) => format!("Move {n} folder(s) to {}? Nothing is deleted; each folder keeps its relative path.", d.display()),
                _ => format!("Send {n} folder(s) to the recycle bin?"),
            };
            st.pending = pending;
            ui.set_pending_action(text.into());
        }
    };
    {
        let state = state.clone();
        let request = request.clone();
        ui.on_request_move(move || {
            let dest = state.lock().unwrap().cfg.cleanup_dir.clone().filter(|d| d.is_dir()).or_else(pick_folder);
            if let Some(d) = dest {
                request(Pending::Move(d));
            }
        });
    }
    ui.on_request_recycle(move || request(Pending::Recycle));
    {
        let state = state.clone();
        let w = ui.as_weak();
        ui.on_cancel_action(move || {
            state.lock().unwrap().pending = Pending::None;
            w.unwrap().set_pending_action("".into());
        });
    }
    {
        let state = state.clone();
        let w = ui.as_weak();
        ui.on_confirm_action(move || {
            let ui = w.unwrap();
            let (dirs, selected, action) = {
                let mut st = state.lock().unwrap();
                let action = match std::mem::replace(&mut st.pending, Pending::None) {
                    Pending::Move(d) => Action::MoveTo(d),
                    Pending::Recycle => Action::RecycleBin,
                    Pending::None => return,
                };
                let selected: Vec<Candidate> = st
                    .cleanup
                    .as_ref()
                    .map(|r| r.candidates.iter().zip(&st.checked).filter(|(_, c)| **c).map(|(c, _)| c.clone()).collect())
                    .unwrap_or_default();
                (st.cfg.game_dirs(), selected, action)
            };
            ui.set_pending_action("".into());
            ui.set_busy(true);
            let state = state.clone();
            let w = w.clone();
            std::thread::spawn(move || {
                let mut cb = progress_cb(w.clone());
                let out = cleanup::apply(&dirs, &selected, &action, &mut cb);
                {
                    // Drop the handled folders from the list.
                    let mut st = state.lock().unwrap();
                    let gone: Vec<&Candidate> = selected.iter().filter(|c| !c.path.exists()).collect();
                    if let Some(r) = st.cleanup.as_mut() {
                        let keep: Vec<bool> = r.candidates.iter().map(|c| !gone.contains(&c)).collect();
                        let mut it = keep.iter();
                        r.candidates.retain(|_| *it.next().unwrap());
                    }
                    let n = st.cleanup.as_ref().map_or(0, |r| r.candidates.len());
                    st.checked = vec![false; n];
                }
                on_ui(&w, move |ui| {
                    let st = state.lock().unwrap();
                    if let Some(r) = &st.cleanup {
                        ui.set_clean_rows(model(clean_rows(r, &st.checked)));
                    }
                    let mut msg = format!("{} folder(s) moved.", out.done.len());
                    if !out.failed.is_empty() {
                        msg += &format!(" {} failed: {}", out.failed.len(), out.failed.iter().map(|(n, e)| format!("{n} ({e})")).collect::<Vec<_>>().join("; "));
                    }
                    ui.set_clean_summary(msg.into());
                    ui.set_progress(1.0);
                    ui.set_status("Done.".into());
                    ui.set_busy(false);
                    update_selected(ui, &st);
                });
            });
        });
    }

    ui.run()
}
