// tauri-nspanel uses the legacy objc crate; suppress its deprecation warnings.
#![allow(deprecated, unexpected_cfgs)]

mod balance;
mod config;
mod model;
mod parser;
mod pricing;
mod store;
pub mod store_codex;

use model::Dashboard;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{
    tray::{MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager,
};
#[cfg(not(target_os = "macos"))]
use tauri::WindowEvent;
use std::time::Duration;
use tauri_plugin_autostart::ManagerExt;
// Positioner is only used for the non-macOS fallback; macOS positions the
// NSPanel manually (see position_panel).
#[cfg(not(target_os = "macos"))]
use tauri_plugin_positioner::{Position, WindowExt};
// NSPanel: lets the popover float over apps in native fullscreen (a plain
// NSWindow from a background/Accessory app cannot overlay another app's
// fullscreen Space). `get_webview_panel` / `to_panel` come from these traits.
#[cfg(target_os = "macos")]
use tauri_nspanel::{ManagerExt as _, WebviewWindowExt as _};

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Rebuild the dashboard (incremental), update the tray's token count, and push
/// the fresh data to the UI so an open popover updates live.
fn refresh(app: &tauri::AppHandle) {
    let dash = parser::build_dashboard_with_mode(utc_day_enabled());
    update_tray_label(app, &dash);
    check_milestones(app, &dash);
    let _ = app.emit("dashboard-updated", &dash);
}

/// Set the tray title + tooltip according to the saved tray mode. macOS shows
/// the label next to the menu-bar icon (set_title); Windows' taskbar tray has
/// no equivalent — set_title is a no-op there — so the same text goes into the
/// hover tooltip, the only text channel Shell_NotifyIcon exposes.
fn update_tray_label(app: &tauri::AppHandle, dash: &Dashboard) {
    let Some(tray) = app.tray_by_id("main") else {
        return;
    };
    let (label, sub) = match load_tray_mode() {
        TrayMode::Tokens => {
            let l = fmt_tokens_m(dash.today_tokens);
            (l.clone(), format!("today {l}"))
        }
        TrayMode::Balance => match balance::balance() {
            Some(b) => {
                let l = fmt_balance_label(&b);
                (l.clone(), format!("balance {l}"))
            }
            None => ("—".to_string(), "balance unavailable".to_string()),
        },
    };
    let _ = tray.set_title(Some(label.clone()));
    let _ = tray.set_tooltip(Some(format!(
        "OCScale v{} · {sub}",
        env!("CARGO_PKG_VERSION")
    )));
}

/// Persisted 100M-token milestone snapshot. Stored in the app *data* dir so it
/// survives app restarts, reboots, and updates (which only replace the .app
/// bundle, never the data dir). The per-period ids let us tell a real crossing
/// from a period reset.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct MilestoneState {
    week_id: String,
    week_floor: i64,
    month_id: String,
    month_floor: i64,
}

/// 100M-token celebration tracking. `state` is the last persisted snapshot
/// (`None` only before the very first observation ever, so the first run
/// baselines without celebrating pre-existing usage). `active` guards against
/// overlapping celebrations.
struct Celebration {
    state: std::sync::Mutex<Option<MilestoneState>>,
    active: AtomicBool,
}

/// `~/Library/Application Support/ocscale/milestones.json` (platform
/// equivalent elsewhere). Deliberately the data dir, not the Caches dir the
/// event store uses — Caches can be purged by the OS, milestones must not be.
fn milestones_path() -> Option<std::path::PathBuf> {
    let dir = dirs::data_dir()?.join("ocscale");
    let _ = std::fs::create_dir_all(&dir);
    Some(dir.join("milestones.json"))
}

fn load_milestones() -> Option<MilestoneState> {
    let t = std::fs::read_to_string(milestones_path()?).ok()?;
    serde_json::from_str(&t).ok()
}

fn save_milestones(m: &MilestoneState) {
    if let Some(p) = milestones_path() {
        if let Ok(t) = serde_json::to_string(m) {
            let _ = std::fs::write(p, t);
        }
    }
}

// ── Launch-at-login preference ──────────────────────────────────────
// Persisted in the data dir (survives restarts/updates, like milestones). The
// on/off toggle lives in the tray's right-click menu; on startup we reconcile
// the OS registration to this preference rather than force-enabling every
// launch (which silently undid a user who had turned autostart off).
fn autostart_pref_path() -> Option<std::path::PathBuf> {
    let dir = dirs::data_dir()?.join("ocscale");
    let _ = std::fs::create_dir_all(&dir);
    Some(dir.join("autostart.json"))
}

fn load_autostart_pref() -> Option<bool> {
    let t = std::fs::read_to_string(autostart_pref_path()?).ok()?;
    serde_json::from_str(&t).ok()
}

fn save_autostart_pref(on: bool) {
    if let Some(p) = autostart_pref_path() {
        if let Ok(t) = serde_json::to_string(&on) {
            let _ = std::fs::write(p, t);
        }
    }
}

/// The LaunchAgent plist the auto-launch plugin writes (product name "OCScale").
#[cfg(target_os = "macos")]
fn autostart_plist_path() -> Option<std::path::PathBuf> {
    Some(dirs::home_dir()?.join("Library/LaunchAgents/OCScale.plist"))
}

/// True when the LaunchAgent plist still points at the currently running
/// binary. The auto-launch plugin only writes the plist and never refreshes
/// launchd, so after the app is renamed or moved the registration silently
/// goes stale (job spawns with EX_CONFIG at login). Re-enabling rewrites the
/// plist with the current exe, which heals this on the next start.
#[cfg(target_os = "macos")]
fn autostart_plist_matches_current_exe() -> bool {
    let Some(path) = autostart_plist_path() else {
        return true; // can't check → don't churn
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return false; // no plist → not registered → will be enabled below
    };
    let Ok(exe) = std::env::current_exe().and_then(|p| p.canonicalize()) else {
        return true;
    };
    text.contains(exe.to_str().unwrap_or(""))
}

// ── Tray label mode (tokens vs balance) ────────────────────────────
// Persisted in the data dir like the autostart preference. Defaults to the
// token count; `set_tray_mode` re-labels the tray immediately.
#[derive(Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum TrayMode {
    Tokens,
    Balance,
}

fn tray_mode_pref_path() -> Option<std::path::PathBuf> {
    let dir = dirs::data_dir()?.join("ocscale");
    let _ = std::fs::create_dir_all(&dir);
    Some(dir.join("tray_mode.json"))
}

fn load_tray_mode() -> TrayMode {
    let Some(p) = tray_mode_pref_path() else {
        return TrayMode::Tokens;
    };
    std::fs::read_to_string(p)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(TrayMode::Tokens)
}

fn save_tray_mode(m: TrayMode) {
    if let Some(p) = tray_mode_pref_path() {
        if let Ok(t) = serde_json::to_string(&m) {
            let _ = std::fs::write(p, t);
        }
    }
}

fn tray_mode_name(m: TrayMode) -> String {
    serde_json::to_string(&m)
        .map(|s| s.trim_matches('"').to_string())
        .unwrap_or_else(|_| "tokens".to_string())
}

// ── Day boundary mode (local calendar day vs UTC "platform day") ─────
// DeepSeek's usage dashboard splits days on UTC, so a session that starts in
// the early local morning lands on the *previous* platform day. The toggle
// switches the Day report + tray "today" between the two boundaries.
#[derive(Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum DayMode {
    Local,
    Utc,
}

fn day_mode_pref_path() -> Option<std::path::PathBuf> {
    let dir = dirs::data_dir()?.join("ocscale");
    let _ = std::fs::create_dir_all(&dir);
    Some(dir.join("day_mode.json"))
}

fn load_day_mode() -> DayMode {
    let Some(p) = day_mode_pref_path() else {
        return DayMode::Local;
    };
    std::fs::read_to_string(p)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(DayMode::Local)
}

fn save_day_mode(m: DayMode) {
    if let Some(p) = day_mode_pref_path() {
        if let Ok(t) = serde_json::to_string(&m) {
            let _ = std::fs::write(p, t);
        }
    }
}

fn day_mode_name(m: DayMode) -> String {
    serde_json::to_string(&m)
        .map(|s| s.trim_matches('"').to_string())
        .unwrap_or_else(|_| "local".to_string())
}

fn utc_day_enabled() -> bool {
    load_day_mode() == DayMode::Utc
}

/// Bring the OS launch-at-login registration in line with the saved preference,
/// returning the effective preference (used to seed the menu checkbox). First
/// run (no saved pref) defaults to on and records it; thereafter we honor the
/// user's choice and only touch the registration when it actually differs.
fn reconcile_autostart(app: &tauri::AppHandle) -> bool {
    let pref = match load_autostart_pref() {
        Some(p) => p,
        None => {
            save_autostart_pref(true);
            true
        }
    };
    let mgr = app.autolaunch();
    let cur = mgr.is_enabled().unwrap_or(false);
    // Re-enable when unregistered OR when the plist points at a different
    // binary (renamed/moved app) — see autostart_plist_matches_current_exe.
    #[cfg(target_os = "macos")]
    let points_at_us = autostart_plist_matches_current_exe();
    #[cfg(not(target_os = "macos"))]
    let points_at_us = true;
    if pref && (!cur || !points_at_us) {
        let _ = mgr.enable();
    } else if !pref && cur {
        let _ = mgr.disable();
    }
    pref
}

/// Current calendar-week and calendar-month identifiers, matching parser.rs's
/// period definitions (Monday-based week, calendar month), so a stored floor is
/// only ever compared within the same period.
fn period_ids() -> (String, String) {
    use chrono::Datelike;
    let d = chrono::Local::now().date_naive();
    let iso = d.iso_week();
    (
        format!("{}-W{:02}", iso.year(), iso.week()),
        format!("{}-{:02}", d.year(), d.month()),
    )
}

/// Decide whether to celebrate: fire if either period advanced to a higher
/// 100M floor *within the same period*. `None` (first ever observation) never
/// fires. A period-id mismatch means that period reset, so it re-baselines
/// silently rather than comparing floors. Returns a single bool, so a jump
/// across several boundaries — or week and month advancing together — is one
/// celebration.
fn milestone_fire(prev: Option<&MilestoneState>, cur: &MilestoneState) -> bool {
    match prev {
        None => false,
        Some(p) => {
            (p.week_id == cur.week_id && cur.week_floor > p.week_floor)
                || (p.month_id == cur.month_id && cur.month_floor > p.month_floor)
        }
    }
}

/// Observe the latest totals, persist the snapshot, and celebrate on a new
/// 100M-token milestone. We watch week ∪ month, not day: today is always within
/// both the current week and month, so a day crossing is already implied by the
/// month — but a calendar week can straddle a month boundary, so early in a
/// month the week total can lead the (freshly reset) month, hence both. Because
/// the snapshot is persisted, a crossing that happened while the app wasn't
/// running (it reads the logs Claude writes regardless) still catches up on the
/// next observation.
fn check_milestones(app: &tauri::AppHandle, dash: &Dashboard) {
    let Some(state) = app.try_state::<Celebration>() else {
        return;
    };
    // total_tokens is already in millions, so a 100M milestone is total / 100.
    let (week_id, month_id) = period_ids();
    let cur = MilestoneState {
        week_id,
        week_floor: (dash.week.metrics.total_tokens / 100.0).floor() as i64,
        month_id,
        month_floor: (dash.month.metrics.total_tokens / 100.0).floor() as i64,
    };

    let mut g = state.state.lock().unwrap();
    let fire = milestone_fire(g.as_ref(), &cur);
    // Keep the persisted floors monotonic within a period: a later observation
    // with a lower total (a transient/partial read, or two observers racing)
    // must not regress the stored floor and re-fire the celebration on restart.
    let mut next = cur.clone();
    if let Some(prev) = g.as_ref() {
        if prev.week_id == next.week_id && prev.week_floor > next.week_floor {
            next.week_floor = prev.week_floor;
        }
        if prev.month_id == next.month_id && prev.month_floor > next.month_floor {
            next.month_floor = prev.month_floor;
        }
    }
    *g = Some(next.clone());
    // Persist while still holding the lock so two observers can't interleave and
    // write a stale snapshot over a newer one.
    save_milestones(&next);
    drop(g);
    if fire {
        celebrate(app);
    }
}

/// Trigger the celebration overlay. Window/panel work must run on the main
/// thread (refresh() runs on a background thread), so hop there.
fn celebrate(app: &tauri::AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || show_celebration(&handle));
}

/// Show (or reuse) a full-screen, click-through, non-activating overlay on the
/// primary monitor and run the confetti animation, then hide it after it plays.
/// Must be called on the main thread.
fn show_celebration(app: &tauri::AppHandle) {
    let Some(state) = app.try_state::<Celebration>() else {
        return;
    };
    // Skip if a celebration is already playing.
    if state.active.swap(true, Ordering::SeqCst) {
        return;
    }

    let (pos, size) = match app.primary_monitor() {
        Ok(Some(m)) => (*m.position(), *m.size()),
        _ => {
            state.active.store(false, Ordering::SeqCst);
            return;
        }
    };

    // Whether the confetti window was reused or freshly built — only used on
    // macOS to decide whether to (re-)apply the NSPanel attributes.
    #[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
    let existed = app.get_webview_window("confetti").is_some();
    let win = match app.get_webview_window("confetti") {
        Some(w) => w,
        None => {
            match tauri::WebviewWindowBuilder::new(
                app,
                "confetti",
                tauri::WebviewUrl::App("confetti.html".into()),
            )
            .title("OCScale Celebration")
            .inner_size(size.width as f64, size.height as f64)
            .decorations(false)
            .transparent(true)
            .shadow(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .focused(false)
            .resizable(false)
            .visible(false)
            .build()
            {
                Ok(w) => w,
                Err(_) => {
                    state.active.store(false, Ordering::SeqCst);
                    return;
                }
            }
        }
    };

    // Cover the whole primary monitor and let clicks pass through to the apps
    // beneath — the celebration must never interrupt what the user is doing.
    let _ = win.set_position(pos);
    let _ = win.set_size(size);
    let _ = win.set_ignore_cursor_events(true);

    #[cfg(target_os = "macos")]
    {
        use tauri_nspanel::cocoa::appkit::NSWindowCollectionBehavior;
        #[allow(non_upper_case_globals)]
        const NS_NONACTIVATING_PANEL: i32 = 1 << 7;

        // Convert to a non-activating panel once, so it can float over apps in
        // native fullscreen without stealing focus (same approach as the main
        // popover). On reuse the window is already a panel.
        if !existed {
            if let Ok(panel) = win.to_panel() {
                panel.set_level(25); // NSMainMenuWindowLevel (24) + 1
                panel.set_style_mask(NS_NONACTIVATING_PANEL);
                panel.set_collection_behaviour(
                    NSWindowCollectionBehavior::NSWindowCollectionBehaviorMoveToActiveSpace
                        | NSWindowCollectionBehavior::NSWindowCollectionBehaviorFullScreenAuxiliary,
                );
            }
        }
        let _ = win.eval("window.__burst&&window.__burst()");
        if let Ok(panel) = app.get_webview_panel("confetti") {
            panel.show();
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = win.eval("window.__burst&&window.__burst()");
        let _ = win.show();
    }

    // Hide once the animation has played out (emission ~2.3s + fall/fade).
    let app2 = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(4200));
        let app3 = app2.clone();
        let _ = app2.run_on_main_thread(move || {
            #[cfg(target_os = "macos")]
            if let Ok(panel) = app3.get_webview_panel("confetti") {
                panel.order_out(None);
            }
            #[cfg(not(target_os = "macos"))]
            if let Some(w) = app3.get_webview_window("confetti") {
                let _ = w.hide();
            }
            if let Some(st) = app3.try_state::<Celebration>() {
                st.active.store(false, Ordering::SeqCst);
            }
        });
    });
}

/// Last tray-icon rectangle (physical px: x, y, width, height), captured on tray
/// click. Used to anchor the panel like tauri-plugin-positioner's
/// TrayBottomCenter — but we can't use the positioner itself on a swizzled
/// NSPanel: its calculate_position calls current_monitor().unwrap(), which fails
/// for a hidden/panel window, so positioning silently no-ops (panel stays
/// top-left). We also must add the icon height ourselves (see position_panel).
///
/// On Windows the cached tray rect is used only to pick which monitor the
/// popover opens on (see position_popover_windows); the popover itself is then
/// pinned to that monitor's top-right work-area corner with a small margin.
struct TrayAnchor(std::sync::Mutex<Option<(f64, f64, f64, f64)>>);

/// Timestamp (ms) of the last drag start. The popover hides on focus loss; on
/// Windows `start_dragging` enters the OS move loop which briefly blurs the
/// window, so we ignore the hide for a short window after a drag.
#[cfg(not(target_os = "macos"))]
struct DragGuard(AtomicI64);

/// Start dragging the borderless popover (Windows/Linux). Done via a command
/// (not the JS drag-region) so we can record the drag start and suppress the
/// imminent hide-on-blur. The frontend only calls this once a real drag begins.
#[cfg(not(target_os = "macos"))]
#[tauri::command]
fn begin_drag(window: tauri::Window) -> Result<(), String> {
    if let Some(g) = window.try_state::<DragGuard>() {
        g.0.store(now_ms(), Ordering::Relaxed);
    }
    window.start_dragging().map_err(|e| e.to_string())
}

/// macOS uses a menu-bar NSPanel that isn't user-draggable, so begin_drag is a
/// no-op there. It's also never invoked (the frontend gates it out) — this just
/// keeps the shared invoke_handler list valid and guarantees zero macOS effect.
#[cfg(target_os = "macos")]
#[tauri::command]
fn begin_drag(_window: tauri::Window) -> Result<(), String> {
    Ok(())
}

/// Anchor the panel under the tray icon, flush with the menu-bar bottom.
#[cfg(target_os = "macos")]
fn position_panel(app: &tauri::AppHandle) {
    let Some(w) = app.get_webview_window("main") else {
        return;
    };
    let Ok(size) = w.outer_size() else {
        return;
    };
    let win_w = size.width as f64;

    if let Some(state) = app.try_state::<TrayAnchor>() {
        if let Some((tx, ty, tw, th)) = *state.0.lock().unwrap() {
            let x = tx + tw / 2.0 - win_w / 2.0;
            let y = ty + th;
            let _ = w.set_position(tauri::PhysicalPosition::new(x as i32, y as i32));
            return;
        }
    }

    // Fallback: centre near the top of the current monitor.
    if let Ok(Some(monitor)) = w.current_monitor() {
        let mp = monitor.position();
        let ms = monitor.size();
        let x = mp.x as f64 + (ms.width as f64 - win_w) / 2.0;
        let y = mp.y as f64 + 24.0 * monitor.scale_factor();
        let _ = w.set_position(tauri::PhysicalPosition::new(x as i32, y as i32));
    }
}

// ── Popover position memory (Windows/Linux) ─────────────────────────
// The borderless popover can be dragged (a header drag region calls
// startDragging in the frontend); we remember where the user left it and reopen
// there next time, falling back to the default top-right when there's no saved
// position on a connected monitor. macOS uses a menu-bar-anchored NSPanel and
// does not persist a position.
#[cfg(not(target_os = "macos"))]
fn popover_pos_path() -> Option<std::path::PathBuf> {
    let dir = dirs::data_dir()?.join("ocscale");
    let _ = std::fs::create_dir_all(&dir);
    Some(dir.join("popover_pos.json"))
}

#[cfg(not(target_os = "macos"))]
fn load_popover_pos() -> Option<(i32, i32)> {
    let t = std::fs::read_to_string(popover_pos_path()?).ok()?;
    serde_json::from_str(&t).ok()
}

#[cfg(not(target_os = "macos"))]
fn save_popover_pos(x: i32, y: i32) {
    if let Some(p) = popover_pos_path() {
        if let Ok(t) = serde_json::to_string(&(x, y)) {
            let _ = std::fs::write(p, t);
        }
    }
}

/// Centre the panel on the current monitor.
#[cfg(not(target_os = "macos"))]
fn position_popover_windows(app: &tauri::AppHandle) {
    const POPOVER_W: f64 = 400.0;
    const POPOVER_H: f64 = 660.0;

    let Some(w) = app.get_webview_window("main") else {
        return;
    };
    let fit = |scale: f64| {
        let _ = w.set_size(tauri::PhysicalSize::new(
            (POPOVER_W * scale).round() as u32,
            (POPOVER_H * scale).round() as u32,
        ));
    };

    let monitor = w.current_monitor().ok().flatten()
        .or_else(|| app.primary_monitor().ok().flatten());

    if let Some(m) = monitor {
        let area = m.work_area();
        let scale = m.scale_factor();
        let win_w = POPOVER_W * scale;
        let win_h = POPOVER_H * scale;
        let x = area.position.x as f64 + (area.size.width as f64 - win_w) / 2.0;
        let y = area.position.y as f64 + (area.size.height as f64 - win_h) / 2.0;
        let _ = w.set_position(tauri::PhysicalPosition::new(
            x.max(0.0) as i32,
            y.max(0.0) as i32,
        ));
        fit(scale);
    } else {
        let _ = w.move_window(Position::TopRight);
    }
}

/// True if our (Accessory) app is currently the frontmost application.
#[cfg(target_os = "macos")]
fn app_is_frontmost() -> bool {
    use tauri_nspanel::cocoa::base::id;
    use tauri_nspanel::objc::{class, msg_send, sel, sel_impl};
    unsafe {
        let proc_info: id = msg_send![class!(NSProcessInfo), processInfo];
        let our_pid: i32 = msg_send![proc_info, processIdentifier];
        let workspace: id = msg_send![class!(NSWorkspace), sharedWorkspace];
        let front: id = msg_send![workspace, frontmostApplication];
        if front.is_null() {
            return false;
        }
        let front_pid: i32 = msg_send![front, processIdentifier];
        front_pid == our_pid
    }
}

/// Hide the panel when the user switches Space or activates another app, so it
/// doesn't linger over the new (e.g. fullscreen) Space until the next click.
/// resign-key alone misses pure Space switches because the panel joins all
/// Spaces and can stay key across the transition.
#[cfg(target_os = "macos")]
fn hide_panel_on_context_switch(app: &tauri::AppHandle) {
    if app_is_frontmost() {
        return;
    }
    if let Ok(panel) = app.get_webview_panel("main") {
        if panel.is_visible() {
            panel.order_out(None);
        }
    }
}

/// Register NSWorkspace observers that auto-hide the panel on Space change / app
/// activation (mirrors tauri-nspanel's menu-bar example). The observers live for
/// the whole app lifetime, so the returned tokens are intentionally dropped.
#[cfg(target_os = "macos")]
fn register_panel_autohide(app: &tauri::AppHandle) {
    use std::ffi::CString;
    use tauri_nspanel::block::ConcreteBlock;
    use tauri_nspanel::cocoa::base::{id, nil};
    use tauri_nspanel::objc::{class, msg_send, sel, sel_impl};

    unsafe {
        let workspace: id = msg_send![class!(NSWorkspace), sharedWorkspace];
        let center: id = msg_send![workspace, notificationCenter];
        for name in [
            "NSWorkspaceActiveSpaceDidChangeNotification",
            "NSWorkspaceDidActivateApplicationNotification",
        ] {
            let app = app.clone();
            let block = ConcreteBlock::new(move |_notif: id| {
                hide_panel_on_context_switch(&app);
            });
            let block = block.copy();
            let ns_name: id = msg_send![
                class!(NSString),
                stringWithUTF8String: CString::new(name).unwrap().as_ptr()
            ];
            let _: id = msg_send![
                center,
                addObserverForName: ns_name object: nil queue: nil usingBlock: block
            ];
        }
    }
}

/// Read the user's GLOBAL macOS appearance preference: true when dark mode is on.
/// We read `AppleInterfaceStyle` from NSUserDefaults (present and "Dark" => dark,
/// absent => light) rather than the app's NSApp.effectiveAppearance — an
/// Accessory (menu-bar) app never becomes frontmost, so its effective appearance
/// (and thus the webview's `prefers-color-scheme`) can lag the real system value.
/// The user default reflects the system setting directly, regardless of focus.
#[cfg(target_os = "macos")]
fn system_is_dark() -> bool {
    use std::ffi::CStr;
    use tauri_nspanel::cocoa::base::{id, nil};
    use tauri_nspanel::objc::{class, msg_send, sel, sel_impl};
    unsafe {
        let defaults: id = msg_send![class!(NSUserDefaults), standardUserDefaults];
        let key: id = msg_send![
            class!(NSString),
            stringWithUTF8String: b"AppleInterfaceStyle\0".as_ptr() as *const std::os::raw::c_char
        ];
        let val: id = msg_send![defaults, stringForKey: key];
        if val == nil {
            return false;
        }
        let raw: *const std::os::raw::c_char = msg_send![val, UTF8String];
        if raw.is_null() {
            return false;
        }
        CStr::from_ptr(raw).to_string_lossy().eq_ignore_ascii_case("dark")
    }
}

/// Watch for live system dark/light-mode changes and push them to the frontend.
/// `AppleInterfaceThemeChangedNotification` is posted on the DISTRIBUTED
/// notification center the instant the user flips Appearance, and is delivered
/// to every registered app regardless of activation policy or frontmost status —
/// so it works for our hidden, non-activating menu-bar panel where the webview's
/// own `prefers-color-scheme` `change` event does not reliably fire. The observer
/// lives for the whole app lifetime, so the returned token is intentionally
/// dropped (same as register_panel_autohide).
#[cfg(target_os = "macos")]
fn watch_system_theme(app: &tauri::AppHandle) {
    use std::ffi::CString;
    use tauri_nspanel::block::ConcreteBlock;
    use tauri_nspanel::cocoa::base::{id, nil};
    use tauri_nspanel::objc::{class, msg_send, sel, sel_impl};

    unsafe {
        let center: id = msg_send![class!(NSDistributedNotificationCenter), defaultCenter];
        let app = app.clone();
        let block = ConcreteBlock::new(move |_notif: id| {
            let _ = app.emit("system-theme", system_is_dark());
        });
        let block = block.copy();
        let ns_name: id = msg_send![
            class!(NSString),
            stringWithUTF8String: CString::new("AppleInterfaceThemeChangedNotification").unwrap().as_ptr()
        ];
        let _: id = msg_send![
            center,
            addObserverForName: ns_name object: nil queue: nil usingBlock: block
        ];
    }
}

/// Show the panel as a popover anchored under the tray icon, and focus it.
/// Always reset the scroll to the top so it doesn't reopen mid-scroll.
fn show_popover(app: &tauri::AppHandle) {
    // On macOS the window is an NSPanel — position it manually, then show()
    // (makes it key and orders it front, incl. over fullscreen Spaces).
    #[cfg(target_os = "macos")]
    {
        position_panel(app);
        if let Ok(panel) = app.get_webview_panel("main") {
            panel.show();
        }
    }
    #[cfg(not(target_os = "macos"))]
    if let Some(w) = app.get_webview_window("main") {
        // Pin the popover to the monitor's top-right corner (see
        // position_popover_windows).
        position_popover_windows(app);
        let _ = w.show();
        let _ = w.set_focus();
    }
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.eval(
            "(function(){var e=document.querySelector('.om-scroll');if(e){e.scrollTop=0;}else{window.scrollTo(0,0);}})()",
        );
    }
}

#[tauri::command]
async fn get_dashboard(app: tauri::AppHandle) -> Dashboard {
    // build_dashboard does blocking IO (reads/writes the cache, parses logs) and
    // holds BUILD_LOCK — running it inline would block the command on the async
    // runtime and, with a large cache, stall the UI. Hop to a blocking worker
    // (the 30s refresh thread already runs the same work off the main thread).
    // Sync the tray label to this freshly-fetched value. The panel refetches
    // the instant it opens, while the tray otherwise only refreshes every 30s —
    // so without this the two could disagree for up to 30s during heavy usage.
    // Done inside spawn_blocking so a cold balance fetch never stalls the
    // async runtime.
    let app2 = app.clone();
    let dash = tauri::async_runtime::spawn_blocking(move || {
        let dash = parser::build_dashboard_with_mode(utc_day_enabled());
        update_tray_label(&app2, &dash);
        dash
    })
    .await
    .unwrap_or_else(|_| parser::build_dashboard_with_mode(utc_day_enabled()));
    check_milestones(&app, &dash);
    dash
}

/// Save a full-panel screenshot (a `data:image/png;base64,...` URL captured in
/// the webview) to the user's Desktop as `OCScale <date> at <time>.png`.
/// DOM rasterization sidesteps macOS Screen Recording permission entirely.
/// Returns the written file path on success.
#[tauri::command]
fn save_screenshot(data_url: String) -> Result<String, String> {
    use base64::Engine;
    let body = data_url
        .strip_prefix("data:image/png;base64,")
        .ok_or_else(|| "expected a data:image/png;base64,... URL".to_string())?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(body.trim())
        .map_err(|e| format!("invalid base64: {e}"))?;

    let dir = dirs::desktop_dir()
        .ok_or_else(|| "could not resolve the Desktop directory".to_string())?;
    let stamp = chrono::Local::now().format("OCScale %Y-%m-%d at %H.%M.%S.png");
    let path = dir.join(stamp.to_string());

    std::fs::write(&path, &bytes).map_err(|e| format!("failed to write file: {e}"))?;
    Ok(path.to_string_lossy().into_owned())
}

/// For CLI/example validation against real logs.
pub fn dashboard_json() -> String {
    serde_json::to_string_pretty(&parser::build_dashboard()).unwrap_or_default()
}

/// Codex feasibility prototype: build a Dashboard from Codex session transcripts
/// (`~/.codex/sessions/**` + `archived_sessions/`). Not wired into the app —
/// run via `cargo run --example dump_codex`.
pub fn codex_dashboard_json() -> String {
    let events = store_codex::load_events();
    serde_json::to_string_pretty(&parser::build_dashboard_from(&events)).unwrap_or_default()
}

// ── Autostart commands (called from the frontend panel) ─────────────

#[tauri::command]
async fn get_autostart(app: tauri::AppHandle) -> Result<bool, String> {
    app.autolaunch().is_enabled().map_err(|e| e.to_string())
}

#[tauri::command]
async fn quit_app(app: tauri::AppHandle) {
    app.exit(0);
}

#[tauri::command]
async fn set_autostart(app: tauri::AppHandle, on: bool) -> Result<bool, String> {
    let mgr = app.autolaunch();
    if on {
        mgr.enable().map_err(|e| e.to_string())?;
    } else {
        mgr.disable().map_err(|e| e.to_string())?;
    }
    let now_on = mgr.is_enabled().map_err(|e| e.to_string())?;
    save_autostart_pref(now_on);
    Ok(now_on)
}

#[tauri::command]
fn get_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// DeepSeek account balance (cached 5 min in `balance.rs`); `null` when the
/// API key / network / endpoint is unavailable.
#[tauri::command]
async fn get_balance() -> Option<balance::BalanceInfo> {
    tauri::async_runtime::spawn_blocking(balance::balance)
        .await
        .unwrap_or(None)
}

/// Current tray label mode: "tokens" or "balance".
#[tauri::command]
fn get_tray_mode() -> String {
    tray_mode_name(load_tray_mode())
}

/// Switch the tray label between today's token count and the DeepSeek balance,
/// persist the choice, and re-label the tray immediately.
#[tauri::command]
async fn set_tray_mode(app: tauri::AppHandle, mode: String) -> Result<String, String> {
    let m = match mode.as_str() {
        "balance" => TrayMode::Balance,
        "tokens" => TrayMode::Tokens,
        _ => return Err("mode must be \"tokens\" or \"balance\"".to_string()),
    };
    save_tray_mode(m);
    // Re-label right away; a cold balance fetch can hit the network, so hop
    // off the async runtime.
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let dash = parser::build_dashboard_with_mode(utc_day_enabled());
        update_tray_label(&app2, &dash);
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(tray_mode_name(m))
}

/// Current day-boundary mode: "local" (calendar day) or "utc" (platform day).
#[tauri::command]
fn get_day_mode() -> String {
    day_mode_name(load_day_mode())
}

/// Switch the Day report + tray "today" between the local calendar day and the
/// UTC day boundary DeepSeek's platform uses, persist, and re-label the tray.
#[tauri::command]
async fn set_day_mode(app: tauri::AppHandle, mode: String) -> Result<String, String> {
    let m = match mode.as_str() {
        "utc" => DayMode::Utc,
        "local" => DayMode::Local,
        _ => return Err("mode must be \"local\" or \"utc\"".to_string()),
    };
    save_day_mode(m);
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let dash = parser::build_dashboard_with_mode(utc_day_enabled());
        update_tray_label(&app2, &dash);
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(day_mode_name(m))
}

/// One period report at an offset (week/month paging: 0 = current, −1 = previous).
#[tauri::command]
async fn get_period(period: String, offset: i64) -> Result<model::PeriodReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        parser::period_report(&period, offset).ok_or_else(|| format!("unknown period: {period}"))
    })
    .await
    .map_err(|e| e.to_string())?
}

fn fmt_tokens_m(m: f64) -> String {
    if m >= 1.0 {
        format!("{:.2}M", m)
    } else {
        let k = (m * 1000.0).round() as i64;
        // no usage yet (e.g. just past midnight) — "0K" reads like "OK", so
        // show a clearer idle label instead.
        if k <= 0 {
            "Ready".to_string()
        } else {
            format!("{k}K")
        }
    }
}

/// Compact tray label for a balance (¥ for CNY, $ for USD, else the code).
fn fmt_balance_label(b: &balance::BalanceInfo) -> String {
    let (sym, v) = match b.currency.as_str() {
        "CNY" => ("¥", b.total_balance),
        "USD" => ("$", b.total_balance),
        other => return format!("{} {:.2}", other, b.total_balance),
    };
    if v >= 10000.0 {
        format!("{sym}{:.1}K", v / 1000.0)
    } else {
        format!("{sym}{v:.2}")
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Tracks when the popover was last hidden, so a click on the tray icon
    // while it's open (which first blurs/hides it) doesn't immediately reopen.
    let last_hidden = Arc::new(AtomicI64::new(0));

    #[allow(unused_mut)]
    let mut builder = tauri::Builder::default()
        // Must be the FIRST plugin: a second launch (e.g. reinstall/relaunch)
        // hands off to the already-running instance and exits, so the menu bar
        // never shows two icons.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            show_popover(app);
        }))
        .plugin(tauri_plugin_positioner::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ));
    // Registers the WebviewPanelManager state used by `to_panel`/`get_webview_panel`.
    #[cfg(target_os = "macos")]
    {
        builder = builder.plugin(tauri_nspanel::init());
    }

    builder
        .invoke_handler(tauri::generate_handler![get_dashboard, save_screenshot, begin_drag, get_autostart, set_autostart, quit_app, get_version, get_balance, get_tray_mode, set_tray_mode, get_day_mode, set_day_mode, get_period])
        .setup(move |app| {
            // Menu-bar–only app: no Dock icon, runs in the background.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            // Holds the latest tray-icon rect so show_popover can anchor the panel.
            // Captured in the tray click handler on every platform — see
            // position_panel (macOS, below the icon) and position_popover_windows
            // (Windows/Linux, above the icon).
            app.manage(TrayAnchor(std::sync::Mutex::new(None)));
            // Drag-start timestamp so a drag doesn't hide the popover (non-macOS).
            #[cfg(not(target_os = "macos"))]
            app.manage(DragGuard(AtomicI64::new(0)));

            // 100M-token celebration tracking. Load the persisted snapshot so
            // milestones survive restarts/reboots/updates; the first run ever
            // (no file) baselines on first observation without celebrating.
            app.manage(Celebration {
                state: std::sync::Mutex::new(load_milestones()),
                active: AtomicBool::new(false),
            });

            // Reconcile launch-at-login with the user's saved preference. The
            // on/off toggle lives in the tray's right-click menu (built below);
            // we do NOT force-enable on every start, which would undo a manual
            // opt-out. `autostart_on` seeds the menu checkbox.
            let _autostart_on = reconcile_autostart(app.handle());

            // Popover behaviour. On macOS, convert the window to a non-activating
            // NSPanel so it can float over apps in native fullscreen, and hide it
            // on resign-key (clicking outside / switching apps) like a popover.
            #[cfg(target_os = "macos")]
            if let Some(window) = app.get_webview_window("main") {
                use tauri_nspanel::cocoa::appkit::NSWindowCollectionBehavior;
                // NSWindowStyleMaskNonActivatingPanel — receive events without
                // activating (stealing focus from) the frontmost app.
                #[allow(non_upper_case_globals)]
                const NS_NONACTIVATING_PANEL: i32 = 1 << 7;

                let lh = last_hidden.clone();
                let handle = app.handle().clone();
                let delegate = tauri_nspanel::panel_delegate!(OCScalePanelDelegate {
                    window_did_resign_key
                });
                delegate.set_listener(Box::new(move |name: String| {
                    if name == "window_did_resign_key" {
                        lh.store(now_ms(), Ordering::Relaxed);
                        if let Ok(panel) = handle.get_webview_panel("main") {
                            panel.order_out(None);
                        }
                    }
                }));

                if let Ok(panel) = window.to_panel() {
                    panel.set_level(25); // NSMainMenuWindowLevel (24) + 1
                    panel.set_style_mask(NS_NONACTIVATING_PANEL);
                    // MoveToActiveSpace: the panel relocates onto whatever Space
                    // is active *when shown* — so it appears over a fullscreen app
                    // if you open it there, but it does NOT live on every Space.
                    // (CanJoinAllSpaces + Stationary made it omnipresent and kept
                    // it painted through transitions, so it lingered/ghosted over
                    // a fullscreen Space even after order_out.) FullScreenAuxiliary
                    // is what actually permits coexisting with a fullscreen window.
                    panel.set_collection_behaviour(
                        NSWindowCollectionBehavior::NSWindowCollectionBehaviorMoveToActiveSpace
                            | NSWindowCollectionBehavior::NSWindowCollectionBehaviorFullScreenAuxiliary,
                    );
                    panel.set_delegate(delegate);
                }

                // Also hide on Space change / app activation, not just resign-key.
                register_panel_autohide(app.handle());

                // Follow the system appearance natively (the webview's
                // prefers-color-scheme is unreliable for a hidden, non-activating
                // menu-bar panel). Watch for live changes, and emit the current
                // value once now so the frontend's System mode starts correct even
                // if the webview reported a stale appearance at launch.
                watch_system_theme(app.handle());
                let _ = app.emit("system-theme", system_is_dark());
            }

            // Non-macOS: keep the plain window, hide on focus loss.
            #[cfg(not(target_os = "macos"))]
            if let Some(win) = app.get_webview_window("main") {
                let w = win.clone();
                let lh = last_hidden.clone();
                win.on_window_event(move |e| match e {
                    WindowEvent::CloseRequested { api, .. } => {
                        api.prevent_close();
                        lh.store(now_ms(), Ordering::Relaxed);
                        if let Ok(p) = w.outer_position() {
                            save_popover_pos(p.x, p.y);
                        }
                        let _ = w.hide();
                    }
                    WindowEvent::Focused(false) => {
                        // A hidden window's "blur" (e.g. the one Windows fires at
                        // startup) carries a meaningless default position — only a
                        // VISIBLE popover the user clicks away from should be saved
                        // and hidden. Without this, startup persisted the OS's
                        // default placement and every open snapped there.
                        if !w.is_visible().unwrap_or(false) {
                            return;
                        }
                        // A title-bar drag momentarily blurs the window (the OS
                        // move loop); don't treat that as a click-away dismiss.
                        let dragging = w
                            .try_state::<DragGuard>()
                            .map(|g| now_ms() - g.0.load(Ordering::Relaxed) < 700)
                            .unwrap_or(false);
                        if dragging {
                            return;
                        }
                        lh.store(now_ms(), Ordering::Relaxed);
                        // Remember where the user left it (dragged or default) so
                        // the next open reuses this spot.
                        if let Ok(p) = w.outer_position() {
                            save_popover_pos(p.x, p.y);
                        }
                        let _ = w.hide();
                    }
                    _ => {}
                });
            }

            // Build the menu-bar tray: app glyph (template icon) + today's tokens.
            let dash = parser::build_dashboard_with_mode(utc_day_enabled());
            let label = fmt_tokens_m(dash.today_tokens);

            let lh_tray = last_hidden.clone();
            let _tray = TrayIconBuilder::with_id("main")
                .icon(tauri::include_image!("icons/tray-icon.png"))
                .icon_as_template(false)
                .title(&label)
                .tooltip(format!(
                    "OCScale v{} · today {}",
                    env!("CARGO_PKG_VERSION"),
                    label
                ))
                .on_tray_icon_event(move |tray, event| {
                    let app = tray.app_handle();
                    // Cache tray icon rect for panel positioning below the icon.
                    if let TrayIconEvent::Click { rect, .. } = &event {
                        if let Some(anchor) = app.try_state::<TrayAnchor>() {
                            let p = rect.position.to_physical::<f64>(1.0);
                            let s = rect.size.to_physical::<f64>(1.0);
                            *anchor.0.lock().unwrap() = Some((p.x, p.y, s.width, s.height));
                        }
                    }
                    // Any click toggles the panel — no menu.
                    if let TrayIconEvent::Click {
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let just_hidden = now_ms() - lh_tray.load(Ordering::Relaxed) < 250;
                        #[cfg(target_os = "macos")]
                        {
                            let visible = app
                                .get_webview_panel("main")
                                .map(|p| p.is_visible())
                                .unwrap_or(false);
                            if visible {
                                if let Ok(p) = app.get_webview_panel("main") {
                                    p.order_out(None);
                                }
                            } else if !just_hidden {
                                show_popover(app);
                            }
                        }
                        #[cfg(not(target_os = "macos"))]
                        {
                            let visible = app
                                .get_webview_window("main")
                                .and_then(|w| w.is_visible().ok())
                                .unwrap_or(false);
                            if visible {
                                if let Some(w) = app.get_webview_window("main") {
                                    let _ = w.hide();
                                }
                            } else if !just_hidden {
                                show_popover(app);
                            }
                        }
                    }
                })
                .build(app)?;

            // Load prices off the main thread (the fetch can block ~20s on a
            // cold/stale cache) and refresh once a day. build_dashboard reads the
            // memoized copy, so neither JSON parsing nor the network ever runs
            // while BUILD_LOCK is held.
            std::thread::spawn(|| {
                pricing::Pricing::reload_shared();
                loop {
                    std::thread::sleep(Duration::from_secs(24 * 60 * 60));
                    pricing::Pricing::reload_shared();
                }
            });

            // Background refresh: keep the tray's token count current and push
            // live updates to an open popover. Cheap thanks to incremental ingest.
            let handle = app.handle().clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(Duration::from_secs(30));
                refresh(&handle);
            });

            // No filesystem watcher needed: the 30s polling loop above is
            // sufficient for OpenCode's SQLite database (local, cheap query).

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn balance_label_formatting() {
        let b = |total: f64, cur: &str| balance::BalanceInfo {
            currency: cur.to_string(),
            total_balance: total,
            granted_balance: 0.0,
            topped_up_balance: total,
        };
        assert_eq!(fmt_balance_label(&b(67.38, "CNY")), "¥67.38");
        assert_eq!(fmt_balance_label(&b(110.0, "USD")), "$110.00");
        assert_eq!(fmt_balance_label(&b(15000.0, "CNY")), "¥15.0K");
        assert_eq!(fmt_balance_label(&b(5.5, "EUR")), "EUR 5.50");
    }

    #[test]
    fn tray_mode_serde_names() {
        assert_eq!(tray_mode_name(TrayMode::Tokens), "tokens");
        assert_eq!(tray_mode_name(TrayMode::Balance), "balance");
    }

    fn ms(wk: &str, wf: i64, mo: &str, mf: i64) -> MilestoneState {
        MilestoneState {
            week_id: wk.into(),
            week_floor: wf,
            month_id: mo.into(),
            month_floor: mf,
        }
    }

    #[test]
    fn first_ever_observation_baselines_without_firing() {
        // No prior snapshot → never celebrate pre-existing usage on first run.
        assert!(!milestone_fire(None, &ms("2026-W24", 3, "2026-06", 3)));
    }

    #[test]
    fn no_change_does_not_fire() {
        let prev = ms("2026-W24", 1, "2026-06", 3);
        assert!(!milestone_fire(Some(&prev), &ms("2026-W24", 1, "2026-06", 3)));
    }

    #[test]
    fn month_crossing_fires() {
        let prev = ms("2026-W24", 1, "2026-06", 3);
        assert!(milestone_fire(Some(&prev), &ms("2026-W24", 1, "2026-06", 4)));
    }

    #[test]
    fn week_crossing_fires_even_when_month_flat() {
        // Early in a month the week (straddling from the previous month) can lead.
        let prev = ms("2026-W24", 0, "2026-06", 0);
        assert!(milestone_fire(Some(&prev), &ms("2026-W24", 1, "2026-06", 0)));
    }

    #[test]
    fn multi_boundary_jump_is_a_single_fire() {
        // 3 → 7 is still one celebration (fire is a bool, not a count).
        let prev = ms("2026-W24", 1, "2026-06", 3);
        assert!(milestone_fire(Some(&prev), &ms("2026-W24", 1, "2026-06", 7)));
    }

    #[test]
    fn new_month_rebaselines_silently() {
        // Period id changed → that period reset; re-baseline, don't compare floors
        // (so a new month opening below last month's floor never fires).
        let prev = ms("2026-W24", 1, "2026-06", 3);
        assert!(!milestone_fire(Some(&prev), &ms("2026-W27", 0, "2026-07", 0)));
    }

    #[test]
    fn new_week_does_not_fire_on_reset() {
        let prev = ms("2026-W24", 2, "2026-06", 3);
        // New week (id changed), month unchanged and flat → no fire.
        assert!(!milestone_fire(Some(&prev), &ms("2026-W25", 0, "2026-06", 3)));
    }
}
