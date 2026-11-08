// No console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod claude;
mod names;
mod parallel;
mod profiles;
mod usage;

use profiles::{Store, Target};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant},
};
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, WindowEvent,
};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_dialog::DialogExt as _;

const PANEL: &str = "panel";
const TRAY: &str = "main";
const WIDTH: f64 = 380.0;
/// A names lookup that found nothing is tried again after this long (a fresh sign-in fills the cache later).
const NAMES_RETRY: Duration = Duration::from_secs(120);

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Settings {
    /// Off once the user unticks "Mit Windows starten"; on by default.
    no_autostart: bool,
    last_folder: Option<String>,
}

struct App {
    store: Store,
    names: Mutex<HashMap<String, names::Info>>,
    tried: Mutex<HashMap<String, Instant>>,
    /// What a running switch is doing.
    busy: Mutex<Option<String>>,
    /// The last switch's error, until the next one.
    error: Mutex<Option<String>>,
    hidden_at: Mutex<Option<Instant>>,
    settings: Mutex<Settings>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AccountView {
    id: String,
    email: String,
    name: String,
    is_current: bool,
    has_profile: bool,
    windows: Vec<usage::Window>,
    measured_at: Option<i64>,
    /// The e-mail the account's terminal login belongs to, once signed in there.
    terminal: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct View {
    found: bool,
    current: Option<String>,
    busy: Option<String>,
    error: Option<String>,
    accounts: Vec<AccountView>,
    now: i64,
}

impl App {
    fn settings_file(&self) -> PathBuf {
        self.store.root.join("app-settings.json")
    }

    fn save_settings(&self) {
        if let Ok(b) = serde_json::to_vec_pretty(&*self.settings.lock().unwrap()) {
            let _ = fs::write(self.settings_file(), b);
        }
    }

    /// Name, e-mail and organization of `id`, from the cache or the account's own web data.
    fn info(&self, id: &str, data_dir: &Path) -> Option<names::Info> {
        if let Some(i) = self.names.lock().unwrap().get(id).filter(|i| !i.org.is_empty()) {
            return Some(i.clone());
        }
        let due = self.tried.lock().unwrap().get(id).map_or(true, |t| t.elapsed() > NAMES_RETRY);
        if due {
            self.tried.lock().unwrap().insert(id.to_string(), Instant::now());
            if let Some(found) = names::find(data_dir, id) {
                let mut names = self.names.lock().unwrap();
                names.insert(id.to_string(), found);
                if let Ok(b) = serde_json::to_vec_pretty(&*names) {
                    let _ = fs::write(self.store.root.join("names.json"), b);
                }
            }
        }
        self.names.lock().unwrap().get(id).cloned()
    }

    fn view(&self) -> View {
        let now = usage::now_ms();
        let user_data = claude::find_user_data();
        let current = user_data.as_deref().and_then(claude::signed_in);
        let mut ids: Vec<String> = current.iter().cloned().collect();
        for id in self.store.saved() {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        let recorded = usage::recorded(&self.store.root);
        let history = user_data.as_deref().map(usage::history).unwrap_or_default();

        let infos: Vec<(String, Option<names::Info>)> = ids
            .iter()
            .map(|id| {
                let dir = match (&user_data, Some(id) == current.as_ref()) {
                    (Some(ud), true) => ud.clone(),
                    _ => self.store.profiles().join(id).join("data"),
                };
                (id.clone(), self.info(id, &dir))
            })
            .collect();
        // The org the account's usage is counted under: the mod knows it for sure, the web data's first membership otherwise.
        let org_of = |id: &str, info: &Option<names::Info>| {
            recorded.get(id).map(|r| r.org.clone()).filter(|o| !o.is_empty()).or_else(|| info.as_ref().map(|i| i.org.clone())).unwrap_or_default()
        };
        let orgs: Vec<String> = infos.iter().map(|(id, i)| org_of(id, i)).collect();

        let accounts = infos
            .iter()
            .zip(&orgs)
            .map(|((id, info), org)| {
                // Desktop's samples name only the org: use them when no other account shares it.
                let sample = (!org.is_empty() && orgs.iter().filter(|o| *o == org).count() == 1).then(|| history.get(org)).flatten();
                let rec = recorded.get(id);
                let windows = usage::windows(rec, sample, now);
                let email = info
                    .as_ref()
                    .map(|i| i.email.clone())
                    .or_else(|| rec.map(|r| r.email.clone()))
                    .filter(|e| !e.is_empty())
                    .unwrap_or_else(|| format!("Account {}", &id[..8]));
                AccountView {
                    id: id.clone(),
                    email,
                    name: info.as_ref().map(|i| i.name.clone()).unwrap_or_default(),
                    is_current: Some(id) == current.as_ref(),
                    has_profile: self.store.has_profile(id),
                    measured_at: windows.iter().filter_map(|w| w.at).max(),
                    windows,
                    terminal: parallel::signed_in_as(id),
                }
            })
            .collect();

        View {
            found: user_data.is_some(),
            current,
            busy: self.busy.lock().unwrap().clone(),
            error: self.error.lock().unwrap().clone(),
            accounts,
            now,
        }
    }
}

/// Recomputes the view: tray tooltip, `view.json` for the mod, the panel if it is open.
fn refresh(app: &AppHandle) -> View {
    let st = app.state::<App>();
    let view = st.view();
    let current = view.accounts.iter().find(|a| a.is_current);
    let tip = match current {
        Some(a) => {
            let figures: Vec<String> = a
                .windows
                .iter()
                .map(|w| format!("{} {}", if w.kind == "five_hour" { "5h" } else { "Woche" }, w.percent.map_or("–".into(), |p| format!("{} %", p.round()))))
                .collect();
            format!("Claude: {}\n{}", a.email, figures.join(" · "))
        }
        None => "Claude Accounts".into(),
    };
    if let Some(tray) = app.tray_by_id(TRAY) {
        let _ = tray.set_tooltip(Some(tip.chars().take(120).collect::<String>()));
    }
    let exe = std::env::current_exe().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
    let mut file = serde_json::to_value(&view).unwrap_or_default();
    file["app"] = serde_json::Value::String(exe);
    let _ = claude::write_json(&st.store.root.join("view.json"), &file);
    let _ = app.emit("view", &view);
    view
}

fn begin_swap(app: &AppHandle, target: Target) -> Result<(), String> {
    let st = app.state::<App>();
    let user_data = claude::find_user_data().ok_or("Claude Desktop nicht gefunden")?;
    let expect = claude::signed_in(&user_data);
    let what = match &target {
        Target::Account(id) => {
            if expect.as_deref() == Some(id.as_str()) {
                return Err("Dieser Account ist schon angemeldet".into());
            }
            let email = st.view().accounts.into_iter().find(|a| &a.id == id).map(|a| a.email).unwrap_or_default();
            format!("Wechsle zu {email} – Claude startet neu …")
        }
        Target::New => "Melde ab für einen neuen Account – Claude startet neu …".into(),
    };
    {
        let mut busy = st.busy.lock().unwrap();
        if busy.is_some() {
            return Err("Ein Wechsel läuft schon".into());
        }
        *busy = Some(what);
        *st.error.lock().unwrap() = None;
    }
    refresh(app);
    let app = app.clone();
    std::thread::spawn(move || {
        let st = app.state::<App>();
        let result = st.store.swap(&user_data, expect.as_deref(), &target, true);
        *st.busy.lock().unwrap() = None;
        *st.error.lock().unwrap() = result.err();
        refresh(&app);
    });
    Ok(())
}

// ---- commands ---------------------------------------------------------------

#[tauri::command]
fn get_view(app: AppHandle) -> View {
    refresh(&app)
}

#[tauri::command]
fn switch_account(app: AppHandle, id: String) -> Result<(), String> {
    if !claude::is_uuid(&id) {
        return Err("ungültige Account-ID".into());
    }
    begin_swap(&app, Target::Account(id))
}

#[tauri::command]
fn add_account(app: AppHandle) -> Result<(), String> {
    begin_swap(&app, Target::New)
}

/// Asks for a folder and opens a terminal there running Claude Code with the account's own login.
#[tauri::command]
async fn open_terminal(app: AppHandle, id: String) -> Result<(), String> {
    if !claude::is_uuid(&id) {
        return Err("ungültige Account-ID".into());
    }
    hide_panel(app.clone());
    let st = app.state::<App>();
    let account = st.view().accounts.into_iter().find(|a| a.id == id).ok_or("unbekannter Account")?;
    let start = st.settings.lock().unwrap().last_folder.clone().map(PathBuf::from).filter(|p| p.is_dir());
    let mut dialog = app.dialog().file().set_title(format!("Ordner für Claude Code ({})", account.email));
    if let Some(dir) = start {
        dialog = dialog.set_directory(dir);
    }
    let Some(folder) = tauri::async_runtime::spawn_blocking(move || dialog.blocking_pick_folder()).await.map_err(|e| e.to_string())? else {
        return Ok(());
    };
    let folder = folder.into_path().map_err(|e| e.to_string())?;
    st.settings.lock().unwrap().last_folder = Some(folder.to_string_lossy().to_string());
    st.save_settings();
    parallel::launch(&id, &account.email, &folder)
}

#[tauri::command]
fn hide_panel(app: AppHandle) {
    if let Some(w) = app.get_webview_window(PANEL) {
        let _ = w.hide();
        *app.state::<App>().hidden_at.lock().unwrap() = Some(Instant::now());
    }
}

/// Sizes the panel to its content and puts it above the tray, in the work area's corner.
#[tauri::command]
fn fit_panel(app: AppHandle, height: f64) {
    let Some(w) = app.get_webview_window(PANEL) else { return };
    let height = height.clamp(120.0, 900.0);
    let _ = w.set_size(LogicalSize::new(WIDTH, height));
    let monitor = w.current_monitor().ok().flatten().or_else(|| w.primary_monitor().ok().flatten());
    if let Some(m) = monitor {
        let scale = m.scale_factor();
        let area = m.work_area();
        let (pw, ph) = ((WIDTH * scale) as i32, (height * scale) as i32);
        let margin = (12.0 * scale) as i32;
        let x = area.position.x + area.size.width as i32 - pw - margin;
        let y = area.position.y + area.size.height as i32 - ph - margin;
        let _ = w.set_position(PhysicalPosition::new(x, y));
    }
}

fn show_panel(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(PANEL) {
        refresh(app);
        let _ = app.emit("shown", ());
        let _ = w.show();
        let _ = w.set_focus();
    }
}

fn toggle_panel(app: &AppHandle) {
    let Some(w) = app.get_webview_window(PANEL) else { return };
    if w.is_visible().unwrap_or(false) {
        hide_panel(app.clone());
        return;
    }
    // A click on the tray icon first takes the focus from an open panel, which hides it: that click means "close".
    let just_hidden = app.state::<App>().hidden_at.lock().unwrap().is_some_and(|t| t.elapsed() < Duration::from_millis(350));
    if !just_hidden {
        show_panel(app);
    }
}

fn set_autostart(app: &AppHandle, on: bool) {
    if cfg!(debug_assertions) {
        return;
    }
    let al = app.autolaunch();
    if on {
        let _ = al.enable();
    } else if al.is_enabled().unwrap_or(false) {
        let _ = al.disable();
    }
}

/// `--switch <uuid>` / `--add` (from the Claude Code mod), `--terminal <uuid> <folder>`, else open the panel unless started `--hidden`.
fn handle_args(app: &AppHandle, args: &[String]) {
    let pos = |flag: &str| args.iter().position(|a| a == flag);
    let result = if let Some(i) = pos("--switch") {
        match args.get(i + 1).filter(|id| claude::is_uuid(id)) {
            Some(id) => begin_swap(app, Target::Account(id.clone())),
            None => Err("--switch braucht eine Account-ID".into()),
        }
    } else if pos("--add").is_some() {
        begin_swap(app, Target::New)
    } else if let Some(i) = pos("--terminal") {
        // `--terminal <uuid> <folder>`: the panel's Terminal button without the folder picker.
        match (args.get(i + 1).filter(|id| claude::is_uuid(id)), args.get(i + 2)) {
            (Some(id), Some(folder)) => {
                let email = app.state::<App>().view().accounts.into_iter().find(|a| &a.id == id).map(|a| a.email).unwrap_or_else(|| id.clone());
                parallel::launch(id, &email, Path::new(folder))
            }
            _ => Err("--terminal braucht eine Account-ID und einen Ordner".into()),
        }
    } else {
        if pos("--hidden").is_none() {
            show_panel(app);
        }
        Ok(())
    };
    if let Err(e) = result {
        *app.state::<App>().error.lock().unwrap() = Some(e);
        show_panel(app);
    }
}

/// Started from inside a job object (a shell in a Claude Code session, an installer
/// run from one), this app would end with that job: closing Claude for a switch
/// ends its job and with it the switch halfway. Starts a copy outside the job
/// instead and returns true, so this one exits before claiming the single instance.
#[cfg(windows)]
fn left_job() -> bool {
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    use windows_sys::Win32::System::{
        JobObjects::IsProcessInJob,
        Threading::{GetCurrentProcess, CREATE_BREAKAWAY_FROM_JOB, CREATE_NO_WINDOW},
    };

    const MARK: &str = "--detached";
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut in_job = 0;
    let checked = unsafe { IsProcessInJob(GetCurrentProcess(), std::ptr::null_mut(), &mut in_job) } != 0;
    if cfg!(debug_assertions) || !checked || in_job == 0 || args.iter().any(|a| a == MARK) {
        return false;
    }
    let Ok(exe) = std::env::current_exe() else { return false };
    // Most jobs let a child break away; Claude's do.
    if Command::new(&exe).args(&args).arg(MARK).creation_flags(CREATE_BREAKAWAY_FROM_JOB).spawn().is_ok() {
        return true;
    }
    // Otherwise WMI starts it, which is never inside the caller's job.
    let line = std::iter::once(exe.to_string_lossy().to_string())
        .chain(args)
        .chain([MARK.to_string()])
        .map(|a| format!("\"{}\"", a.replace('"', "")))
        .collect::<Vec<_>>()
        .join(" ");
    let command = format!(
        "$r = Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{{ CommandLine = '{}' }}; exit $r.ReturnValue",
        line.replace('\'', "''")
    );
    Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &command])
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .is_ok_and(|s| s.success())
}

#[cfg(not(windows))]
fn left_job() -> bool {
    false
}

fn main() {
    if left_job() {
        return;
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _| handle_args(app, &args)))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, Some(vec!["--hidden"])))
        .invoke_handler(tauri::generate_handler![get_view, switch_account, add_account, open_terminal, hide_panel, fit_panel])
        .setup(|app| {
            let root = claude::local_app_data().join("ClaudeAccountSwitcher");
            fs::create_dir_all(root.join("profiles"))?;
            fs::create_dir_all(root.join("limits"))?;
            let names: HashMap<String, names::Info> = fs::read(root.join("names.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
            let settings: Settings = fs::read(root.join("app-settings.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
            let autostart = !settings.no_autostart;
            app.manage(App {
                store: Store { root },
                names: Mutex::new(names),
                tried: Mutex::default(),
                busy: Mutex::default(),
                error: Mutex::default(),
                hidden_at: Mutex::default(),
                settings: Mutex::new(settings),
            });
            set_autostart(app.handle(), autostart);

            let open_item = MenuItem::with_id(app, "open", "Accounts öffnen", true, None::<&str>)?;
            let auto_item = CheckMenuItem::with_id(app, "autostart", "Mit Windows starten", true, autostart, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Beenden", true, None::<&str>)?;
            TrayIconBuilder::with_id(TRAY)
                .icon(app.default_window_icon().cloned().ok_or("icon missing")?)
                .tooltip("Claude Accounts")
                .menu(&Menu::with_items(app, &[&open_item, &auto_item, &quit_item])?)
                .show_menu_on_left_click(false)
                .on_menu_event(move |app, e| match e.id.as_ref() {
                    "open" => show_panel(app),
                    "autostart" => {
                        let st = app.state::<App>();
                        let on = {
                            let mut s = st.settings.lock().unwrap();
                            s.no_autostart = !s.no_autostart;
                            !s.no_autostart
                        };
                        st.save_settings();
                        set_autostart(app, on);
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, e| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                        toggle_panel(tray.app_handle());
                    }
                })
                .build(app)?;

            let handle = app.handle().clone();
            std::thread::spawn(move || loop {
                refresh(&handle);
                std::thread::sleep(Duration::from_secs(30));
            });
            let args: Vec<String> = std::env::args().collect();
            handle_args(app.handle(), &args);
            Ok(())
        })
        .on_window_event(|window, event| match event {
            WindowEvent::Focused(false) if window.label() == PANEL => hide_panel(window.app_handle().clone()),
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                hide_panel(window.app_handle().clone());
            }
            _ => {}
        })
        .run(tauri::generate_context!())
        .expect("error while running Claude Accounts");
}