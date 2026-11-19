// OMSI-Tools-NG-rs — derived from OMSI-Tools by pietr26 (GPLv2). See LICENSE / README.
#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

mod i18n;

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use omsi_tools_core::cleanup::{self, Action, Candidate, CleanupReport};
use omsi_tools_core::maps::{list_maps, MapEntry};
use omsi_tools_core::scanner::{scan_map, FileKind, Phase, Progress, ScanReport};
use omsi_tools_core::Config;
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel, Weak};

use i18n::{t, tf, Lang};

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
    /// Checked flag per candidate of `cleanup` (same indices).
    checked: Vec<bool>,
    /// Candidate indices shown in the current cleanup tab.
    view: Vec<usize>,
    pending: Pending,
    cancel: Arc<AtomicBool>,
}

type Shared = Arc<Mutex<State>>;

const CLEAN_ROOTS: [&str; 3] = ["Sceneryobjects", "Splines", "Vehicles"];

/// Kinds shown in the overview and in tabs 1..=6.
const TAB_KINDS: [FileKind; 6] = [
    FileKind::Tile,
    FileKind::Texture,
    FileKind::Sceneryobject,
    FileKind::Spline,
    FileKind::Vehicle,
    FileKind::Human,
];

fn kind_label(k: FileKind) -> String {
    t(k.label())
}

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
    st.cfg.save(path).err().map(|e| tf("Could not save settings: {0}", &[&e]))
}

fn model<T: Clone + 'static>(v: Vec<T>) -> ModelRc<T> {
    ModelRc::from(Rc::new(VecModel::from(v)))
}

// ---------------------------------------------------------------- map verification

fn load_picture(map: Option<&MapEntry>) -> Option<slint::Image> {
    let dir = &map?.dir;
    ["picture.jpg", "picture.jpeg", "picture.png", "picture.bmp"]
        .iter()
        .find_map(|n| omsi_tools_core::paths::resolve_existing(dir, n))
        .and_then(|p| slint::Image::load_from_path(&p).ok())
}

fn show_picture(ui: &MainWindow, map: Option<&MapEntry>) {
    match load_picture(map) {
        Some(img) => {
            ui.set_map_picture(img);
            ui.set_has_picture(true);
        }
        None => ui.set_has_picture(false),
    }
}

fn clear_results(ui: &MainWindow) {
    let rows = TAB_KINDS
        .iter()
        .map(|&k| CountRow { name: kind_label(k).into(), total: "–".into(), missing: "–".into(), bad: false })
        .collect();
    ui.set_counts(model(rows));
    ui.set_tab_rows(model(vec![]));
    ui.set_summary("".into());
}

fn reload_maps(ui: &MainWindow, st: &mut State) {
    st.maps = list_maps(&st.cfg.game_dirs());
    let names: Vec<SharedString> = st.maps.iter().map(|m| m.label().into()).collect();
    ui.set_map_names(model(names));
    st.report = None;
    clear_results(ui);
    ui.set_selected_map(if st.maps.is_empty() { -1 } else { 0 });
    show_picture(ui, st.maps.first());
}

fn count_rows(r: &ScanReport) -> Vec<CountRow> {
    TAB_KINDS
        .iter()
        .map(|&k| {
            let missing = r.missing_of(k).count();
            CountRow {
                name: kind_label(k).into(),
                total: r.total_of(k).to_string().into(),
                missing: missing.to_string().into(),
                bad: missing > 0,
            }
        })
        .collect()
}

/// Rows of tab `tab` (1..=6 = one file kind).
fn tab_rows(r: &ScanReport, tab: i32, show_found: bool) -> Vec<ResultRow> {
    let Some(&kind) = usize::try_from(tab - 1).ok().and_then(|i| TAB_KINDS.get(i)) else {
        return Vec::new();
    };
    r.entries
        .iter()
        .filter(|e| e.kind == kind && (e.is_missing() || show_found))
        .map(|e| {
            let by = tf("referenced by: {0}", &[&e.referenced_by.join(", ")]);
            let detail = match &e.found {
                Some((src, _)) => format!("{} · {by}", tf("found in {0}", &[src])),
                None => by,
            };
            ResultRow { header: false, missing: e.is_missing(), text: e.path.clone().into(), detail: detail.into() }
        })
        .collect()
}

fn summary_text(r: &ScanReport) -> String {
    let missing = r.missing_count();
    let mut s = if missing == 0 {
        tf("{0} [{1}]: no missing files among {2} referenced files", &[&r.map_name, &r.source, &r.entries.len()])
    } else {
        tf("{0} [{1}]: {2} missing file(s) among {3} referenced files", &[&r.map_name, &r.source, &missing, &r.entries.len()])
    };
    if r.cancelled {
        s.push(' ');
        s += &t("(scan cancelled, partial result)");
    }
    s
}

fn show_scan(ui: &MainWindow, r: &ScanReport) {
    ui.set_counts(model(count_rows(r)));
    ui.set_tab_rows(model(tab_rows(r, ui.get_tab(), ui.get_show_found())));
    ui.set_summary(summary_text(r).into());
}

// ---------------------------------------------------------------- cleanup

fn root_index(c: &Candidate) -> usize {
    CLEAN_ROOTS.iter().position(|r| r.eq_ignore_ascii_case(&c.root)).unwrap_or(0)
}

fn clean_row(c: &Candidate, checked: bool) -> CleanRow {
    CleanRow {
        text: format!("{}  [{}]", c.relative, c.source).into(),
        detail: tf("{0} files · {1} · {2}", &[&c.file_count, &human_size(c.size_bytes), &c.path.display()]).into(),
        checked,
    }
}

fn update_selected(ui: &MainWindow, st: &State) {
    let Some(r) = &st.cleanup else {
        ui.set_clean_selected("".into());
        return;
    };
    let sel: Vec<&Candidate> = r.candidates.iter().zip(&st.checked).filter(|(_, c)| **c).map(|(c, _)| c).collect();
    let size: u64 = sel.iter().map(|c| c.size_bytes).sum();
    ui.set_clean_selected(tf("{0} folder(s) selected · {1}", &[&sel.len(), &human_size(size)]).into());
}

/// Rebuilds the list of the current cleanup tab plus the tab badges.
fn refresh_clean(ui: &MainWindow, st: &mut State) {
    let tab = usize::try_from(ui.get_clean_tab()).unwrap_or(0);
    match &st.cleanup {
        Some(r) => {
            st.view = (0..r.candidates.len()).filter(|&i| root_index(&r.candidates[i]) == tab).collect();
            let rows: Vec<CleanRow> = st.view.iter().map(|&i| clean_row(&r.candidates[i], st.checked[i])).collect();
            ui.set_clean_rows(model(rows));
            let badges: Vec<SharedString> = (0..3)
                .map(|t| {
                    let n = r.candidates.iter().filter(|c| root_index(c) == t).count();
                    if n == 0 { SharedString::new() } else { n.to_string().into() }
                })
                .collect();
            ui.set_clean_badges(model(badges));
        }
        None => {
            st.view.clear();
            ui.set_clean_rows(model(vec![]));
            ui.set_clean_badges(model(vec![SharedString::new(); 3]));
        }
    }
    update_selected(ui, st);
}

fn clean_summary(r: &CleanupReport) -> String {
    if r.cancelled {
        return t("Analysis cancelled.");
    }
    let total: u64 = r.candidates.iter().map(|c| c.size_bytes).sum();
    tf("{0} unused folder(s), {1} in total — {2} maps checked. Nothing has been changed.", &[&r.candidates.len(), &human_size(total), &r.maps_scanned])
}

// ---------------------------------------------------------------- misc helpers

fn progress_text(p: &Progress) -> String {
    match p.phase {
        Phase::Tile => tf("Reading tile {0} of {1}", &[&(p.done + 1), &p.total]),
        Phase::Lists => t("Reading parklists, humans and AI lists"),
        Phase::Map => tf("Reading map {0} of {1}: {2}", &[&(p.done + 1), &p.total.saturating_sub(2), &p.message]),
        Phase::Refs => t("Checking references between folders"),
        Phase::Measure => t("Measuring unused folders"),
        Phase::Apply => tf("Processing {0}", &[&p.message]),
        Phase::Done => t("Done."),
    }
}

fn on_ui<F: FnOnce(&MainWindow) + Send + 'static>(w: &Weak<MainWindow>, f: F) {
    let _ = w.upgrade_in_event_loop(move |ui| f(&ui));
}

fn progress_cb(w: Weak<MainWindow>) -> impl FnMut(Progress) {
    move |p: Progress| {
        let frac = if p.total == 0 { 0.0 } else { p.done as f32 / p.total as f32 };
        let text = progress_text(&p);
        on_ui(&w, move |ui| {
            ui.set_progress(frac);
            ui.set_status(text.into());
        });
    }
}

fn pick_folder() -> Option<PathBuf> {
    rfd::FileDialog::new().pick_folder()
}

fn set_language(ui: &MainWindow, lang: Lang) {
    i18n::set_lang(lang);
    if let Err(e) = slint::select_bundled_translation(lang.code()) {
        eprintln!("translation {}: {e}", lang.code());
    }
    ui.set_lang_index(Lang::ALL.iter().position(|l| *l == lang).unwrap_or(0) as i32);
}

/// Re-renders everything that was built from translated Rust strings.
fn retranslate(ui: &MainWindow, st: &mut State) {
    match &st.report {
        Some(r) => show_scan(ui, r),
        None => clear_results(ui),
    }
    ui.set_clean_summary(st.cleanup.as_ref().map(clean_summary).unwrap_or_default().into());
    ui.set_pending_action("".into());
    st.pending = Pending::None;
    ui.set_status("".into());
    refresh_clean(ui, st);
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = MainWindow::new()?;

    let cfg_path = Config::default_path();
    let (cfg, load_err) = match cfg_path.as_deref().map(Config::load) {
        Some(Ok(c)) => (c, None),
        Some(Err(e)) => (Config::default(), Some(e.to_string())),
        None => (Config::default(), None),
    };
    // English is the default language.
    set_language(&ui, cfg.language.as_deref().map(Lang::from_code).unwrap_or(Lang::En));
    let state: Shared = Arc::new(Mutex::new(State {
        cfg,
        cfg_path,
        maps: vec![],
        report: None,
        cleanup: None,
        checked: vec![],
        view: vec![],
        pending: Pending::None,
        cancel: Arc::new(AtomicBool::new(false)),
    }));

    ui.set_language_names(model(Lang::ALL.iter().map(|l| SharedString::from(l.native_name())).collect()));
    {
        let mut st = state.lock().unwrap();
        ui.set_omsi_dir(path_str(&st.cfg.omsi_dir));
        ui.set_neo_dir(path_str(&st.cfg.neo_omsi_dir));
        ui.set_open_dir(path_str(&st.cfg.open_omsi_dir));
        ui.set_cleanup_dir(path_str(&st.cfg.cleanup_dir));
        ui.set_config_path(st.cfg_path.as_deref().map(Path::display).map(|d| d.to_string()).unwrap_or_default().into());
        if let Some(d) = st.cfg.dark_theme {
            ui.global::<Theme>().invoke_set_dark(d);
        }
        ui.set_progress(0.0);
        ui.set_status(load_err.map(|e| tf("Could not load settings: {0}", &[&e])).unwrap_or_default().into());
        if st.cfg.game_dirs().is_empty() {
            ui.set_page(3);
            ui.set_status(t("Select your OMSI 2 directory to get started.").into());
        }
        reload_maps(&ui, &mut st);
        refresh_clean(&ui, &mut st);
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
            st.cleanup = None;
            st.checked.clear();
            ui.set_clean_summary("".into());
            refresh_clean(&ui, &mut st);
            ui.set_status(err.unwrap_or_else(|| t("Settings saved.")).into());
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
        ui.on_language_changed(move |idx| {
            let ui = w.unwrap();
            let lang = usize::try_from(idx).ok().and_then(|i| Lang::ALL.get(i).copied()).unwrap_or(Lang::En);
            set_language(&ui, lang);
            let mut st = state.lock().unwrap();
            st.cfg.language = Some(lang.code().to_string());
            let err = save_cfg(&st);
            retranslate(&ui, &mut st);
            if let Some(e) = err {
                ui.set_status(e.into());
            }
        });
    }
    {
        let state = state.clone();
        let w = ui.as_weak();
        ui.on_reload_maps(move || {
            let ui = w.unwrap();
            reload_maps(&ui, &mut state.lock().unwrap());
        });
    }
    {
        let state = state.clone();
        ui.on_theme_saved(move |dark| {
            let mut st = state.lock().unwrap();
            st.cfg.dark_theme = Some(dark);
            save_cfg(&st);
        });
    }

    // --- map verification ---
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
            clear_results(&ui);
            ui.set_status(tf("Scanning {0} …", &[&map.label()]).into());

            let state = state.clone();
            let w = w.clone();
            std::thread::spawn(move || {
                let mut cb = progress_cb(w.clone());
                let report = scan_map(&dirs, &map, &cancel, &mut cb);
                state.lock().unwrap().report = Some(report.clone());
                on_ui(&w, move |ui| {
                    show_scan(ui, &report);
                    ui.set_progress(1.0);
                    ui.set_status(if report.cancelled { t("Scan cancelled.") } else { t("Scan finished.") }.into());
                    ui.set_busy(false);
                });
            });
        });
    }
    {
        let state = state.clone();
        let w = ui.as_weak();
        ui.on_tab_changed(move |tab| {
            let ui = w.unwrap();
            let rows = state.lock().unwrap().report.as_ref().map(|r| tab_rows(r, tab, ui.get_show_found())).unwrap_or_default();
            ui.set_tab_rows(model(rows));
        });
    }
    {
        let state = state.clone();
        let w = ui.as_weak();
        ui.on_show_found_changed(move |show| {
            let ui = w.unwrap();
            if let Some(r) = &state.lock().unwrap().report {
                ui.set_tab_rows(model(tab_rows(r, ui.get_tab(), show)));
            }
        });
    }
    {
        let state = state.clone();
        let w = ui.as_weak();
        ui.on_map_selected(move |idx| {
            let ui = w.unwrap();
            let st = state.lock().unwrap();
            show_picture(&ui, usize::try_from(idx).ok().and_then(|i| st.maps.get(i)));
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
                ui.set_status(t("Configure at least the OMSI 2 directory first.").into());
                return;
            }
            ui.set_busy(true);
            ui.set_pending_action("".into());
            ui.set_progress(0.0);
            ui.set_clean_summary("".into());
            {
                let mut st = state.lock().unwrap();
                st.cleanup = None;
                st.checked.clear();
                refresh_clean(&ui, &mut st);
            }
            let state = state.clone();
            let w = w.clone();
            std::thread::spawn(move || {
                let mut cb = progress_cb(w.clone());
                let report = cleanup::analyze(&dirs, &maps, &cancel, &mut cb);
                {
                    let mut st = state.lock().unwrap();
                    st.checked = vec![false; report.candidates.len()];
                    st.cleanup = Some(report.clone());
                }
                on_ui(&w, move |ui| {
                    let mut st = state.lock().unwrap();
                    ui.set_clean_summary(clean_summary(&report).into());
                    refresh_clean(ui, &mut st);
                    ui.set_progress(1.0);
                    ui.set_status(
                        if report.warnings.is_empty() { t("Analysis finished.") } else { tf("Analysis finished with {0} warning(s).", &[&report.warnings.len()]) }.into(),
                    );
                    ui.set_busy(false);
                });
            });
        });
    }
    {
        let state = state.clone();
        let w = ui.as_weak();
        ui.on_clean_tab_changed(move |_| {
            let ui = w.unwrap();
            refresh_clean(&ui, &mut state.lock().unwrap());
        });
    }
    {
        let state = state.clone();
        let w = ui.as_weak();
        ui.on_clean_toggled(move |i, on| {
            let ui = w.unwrap();
            let mut st = state.lock().unwrap();
            let Some(&g) = usize::try_from(i).ok().and_then(|i| st.view.get(i)) else { return };
            st.checked[g] = on;
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
            for g in st.view.clone() {
                st.checked[g] = on;
            }
            refresh_clean(&ui, &mut st);
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
                ui.set_status(t("Select at least one folder first.").into());
                return;
            }
            let text = match &pending {
                Pending::Move(d) => tf("Move {0} folder(s) to {1}? Nothing is deleted; each folder keeps its relative path.", &[&n, &d.display()]),
                _ => tf("Send {0} folder(s) to the recycle bin?", &[&n]),
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
                    // Drop the handled folders from the lists.
                    let mut st = state.lock().unwrap();
                    if let Some(r) = st.cleanup.as_mut() {
                        r.candidates.retain(|c| !(selected.contains(c) && !c.path.exists()));
                    }
                    let n = st.cleanup.as_ref().map_or(0, |r| r.candidates.len());
                    st.checked = vec![false; n];
                }
                on_ui(&w, move |ui| {
                    let mut st = state.lock().unwrap();
                    let mut msg = tf("{0} folder(s) moved.", &[&out.done.len()]);
                    if !out.failed.is_empty() {
                        let list = out.failed.iter().map(|(n, e)| format!("{n} ({e})")).collect::<Vec<_>>().join("; ");
                        msg.push(' ');
                        msg += &tf("{0} failed: {1}", &[&out.failed.len(), &list]);
                    }
                    ui.set_clean_summary(msg.into());
                    refresh_clean(ui, &mut st);
                    ui.set_progress(1.0);
                    ui.set_status(t("Done.").into());
                    ui.set_busy(false);
                });
            });
        });
    }

    ui.run()
}
