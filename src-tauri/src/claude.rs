//! Claude Desktop on disk and as a process: its data folder, the signed-in
//! account, closing and starting it.

use serde_json::Value;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    thread::sleep,
    time::{Duration, Instant},
};

pub fn is_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36
        && b.iter().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => *c == b'-',
            _ => c.is_ascii_digit() || (b'a'..=b'f').contains(c),
        })
}

pub fn local_app_data() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_default()
}

pub fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_slice(&read_shared(path).ok()?).ok()
}

/// Reads a file another process may hold open (Claude keeps some open).
pub fn read_shared(path: &Path) -> std::io::Result<Vec<u8>> {
    #[cfg(windows)]
    {
        use std::{io::Read, os::windows::fs::OpenOptionsExt};
        // FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE
        let mut f = fs::OpenOptions::new().read(true).share_mode(7).open(path)?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;
        Ok(buf)
    }
    #[cfg(not(windows))]
    fs::read(path)
}

/// Writes JSON next to `path` first and swaps it in, so a crash never leaves half a file.
pub fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    let tmp = path.with_extension("switcher-tmp");
    let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    fs::write(&tmp, bytes).map_err(|e| format!("{}: {e}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Claude Desktop's data folder: `%LOCALAPPDATA%\Claude-Data\claude_<publisher id>`,
/// the one whose config.json changed last if there are several.
pub fn find_user_data() -> Option<PathBuf> {
    let base = local_app_data().join("Claude-Data");
    fs::read_dir(base)
        .ok()?
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().to_ascii_lowercase().starts_with("claude_"))
        .filter_map(|e| {
            let changed = fs::metadata(e.path().join("config.json")).ok()?.modified().ok()?;
            Some((changed, e.path()))
        })
        .max_by_key(|(t, _)| *t)
        .map(|(_, p)| p)
}

/// The account signed in to Claude Desktop, by its uuid.
pub fn signed_in(user_data: &Path) -> Option<String> {
    let config = read_json(&user_data.join("config.json"))?;
    let id = config.get("lastKnownAccountUuid")?.as_str()?;
    (config.get("oauth:tokenCacheV2").is_some() && is_uuid(id)).then(|| id.to_string())
}

/// `Claude_<publisher id>`, the package family the data folder is named after.
fn package_family(user_data: &Path) -> String {
    let leaf = user_data.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    format!("Claude_{}", leaf.trim_start_matches("claude_").trim_start_matches("Claude_"))
}

pub fn start(user_data: &Path) -> Result<(), String> {
    Command::new("explorer.exe")
        .arg(format!("shell:AppsFolder\\{}!Claude", package_family(user_data)))
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Claude starten: {e}"))
}

/// Starts a further Claude Desktop on its own data folder, next to the one on the default
/// folder (`main`, which names the package). Claude keeps a `--user-data-dir` it is started
/// with ("chosen folder") and holds its single-instance lock per folder; started again on a
/// folder already running, it brings that window to the front.
pub fn start_instance(main: &Path, user_data: &Path) -> Result<(), String> {
    let arg = format!("--user-data-dir=\"{}\"", user_data.display());
    win::activate(&format!("{}!Claude", package_family(main)), &arg).map_err(|e| format!("Claude starten: {e}"))
}

/// Claude's own data folders under `%LOCALAPPDATA%\Claude-Data`, which a Desktop started without `--user-data-dir` uses.
fn is_default(user_data: &Path) -> bool {
    norm(&user_data.to_string_lossy()).starts_with(&norm(&local_app_data().join("Claude-Data").to_string_lossy()))
}

fn norm(path: &str) -> String {
    path.replace('/', "\\").trim_end_matches('\\').to_ascii_lowercase()
}

/// The `--user-data-dir` a command line names.
fn data_dir_arg(cmdline: &str) -> Option<String> {
    const FLAG: &str = "--user-data-dir=";
    let rest = &cmdline[cmdline.find(FLAG)? + FLAG.len()..];
    let value = match rest.strip_prefix('"') {
        Some(quoted) => quoted.split('"').next()?,
        None => rest.split(' ').next()?,
    };
    (!value.is_empty()).then(|| norm(value))
}

/// Where a Claude process belongs: the data folder it works on, `None` for the default one.
/// An engine by the folder it was installed into, a Desktop process by its `--user-data-dir`,
/// any other by the process that started it.
fn owner(p: &win::Process, by_id: &HashMap<u32, &win::Process>) -> Option<String> {
    let mut at = p;
    for _ in 0..8 {
        let path = at.path.to_ascii_lowercase();
        if let Some(i) = path.find("\\claude-code\\") {
            return Some(norm(&at.path[..i]));
        }
        if let Some(dir) = data_dir_arg(&at.cmdline) {
            return Some(dir);
        }
        match by_id.get(&at.parent) {
            Some(parent) if parent.id != at.id => at = parent,
            _ => break,
        }
    }
    None
}

/// The processes of the Claude Desktop on `user_data` and its Code sessions' engines: (pid, parent pid).
/// Instances on other data folders are not among them.
fn claude_processes(user_data: &Path) -> Vec<(u32, u32)> {
    let procs: Vec<win::Process> = win::processes("claude.exe")
        .into_iter()
        .filter(|p| {
            let path = p.path.to_ascii_lowercase();
            path.contains("\\windowsapps\\claude_") || path.contains("\\claude-code\\")
        })
        .collect();
    let by_id: HashMap<u32, &win::Process> = procs.iter().map(|p| (p.id, p)).collect();
    let me = norm(&user_data.to_string_lossy());
    let default = is_default(user_data);
    procs
        .iter()
        .filter(|p| match owner(p, &by_id) {
            Some(dir) => dir == me,
            None => default,
        })
        .map(|p| (p.id, p.parent))
        .collect()
}

/// Whether a Claude Desktop runs on `user_data`.
pub fn is_running(user_data: &Path) -> bool {
    !claude_processes(user_data).is_empty()
}

fn claude_pids(user_data: &Path) -> Vec<u32> {
    claude_processes(user_data).into_iter().map(|(id, _)| id).collect()
}

/// Closes Claude Desktop the way Windows does for an update or sign-out, so it
/// saves its state and ends its sessions; ends whatever is left after that.
///
/// A window close only hides Claude in the tray, and ending it outright makes
/// the next start hang on a white window while it recovers.
pub fn stop(user_data: &Path) -> Result<(), String> {
    let running = claude_processes(user_data);
    if running.is_empty() {
        return Ok(());
    }
    let ids: Vec<u32> = running.iter().map(|(id, _)| *id).collect();
    // The app itself: the Claude processes no other Claude process started.
    let apps: Vec<u32> = running.iter().filter(|(_, parent)| !ids.contains(parent)).map(|(id, _)| *id).collect();
    win::request_quit(&apps);
    if !wait_gone(user_data, Duration::from_secs(30)) {
        for id in claude_pids(user_data) {
            win::end(id);
        }
        if !wait_gone(user_data, Duration::from_secs(10)) {
            return Err("Claude liess sich nicht beenden".into());
        }
    }
    // Let the OS release the file handles.
    sleep(Duration::from_millis(800));
    Ok(())
}

fn wait_gone(user_data: &Path, max: Duration) -> bool {
    let until = Instant::now() + max;
    while Instant::now() < until {
        if claude_pids(user_data).is_empty() {
            return true;
        }
        sleep(Duration::from_millis(250));
    }
    claude_pids(user_data).is_empty()
}

#[cfg(windows)]
mod win {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, FILETIME, INVALID_HANDLE_VALUE},
        System::{
            Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS},
            RestartManager::{RmEndSession, RmRegisterResources, RmShutdown, RmStartSession, CCH_RM_SESSION_KEY, RM_UNIQUE_PROCESS},
            Threading::{GetProcessTimes, OpenProcess, QueryFullProcessImageNameW, TerminateProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE},
        },
    };

    pub struct Process {
        pub id: u32,
        pub parent: u32,
        pub path: String,
        pub cmdline: String,
    }

    /// The process's command line (empty where Windows does not hand it out).
    fn command_line(pid: u32) -> String {
        use windows_sys::Wdk::System::Threading::{NtQueryInformationProcess, ProcessCommandLineInformation};
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return String::new();
            }
            let mut need = 0u32;
            NtQueryInformationProcess(h, ProcessCommandLineInformation, std::ptr::null_mut(), 0, &mut need);
            let mut out = String::new();
            if need > 0 && need < 1 << 20 {
                // u64s keep the UNICODE_STRING header at the start aligned.
                let mut buf = vec![0u64; (need as usize).div_ceil(8)];
                if NtQueryInformationProcess(h, ProcessCommandLineInformation, buf.as_mut_ptr().cast(), need, &mut need) >= 0 {
                    let s = &*(buf.as_ptr() as *const windows_sys::Win32::Foundation::UNICODE_STRING);
                    if !s.Buffer.is_null() {
                        out = String::from_utf16_lossy(std::slice::from_raw_parts(s.Buffer, s.Length as usize / 2));
                    }
                }
            }
            CloseHandle(h);
            out
        }
    }

    /// Starts a packaged app with these arguments, as Windows starts it from the Start menu
    /// but with a command line. On a thread of its own in a single-threaded COM apartment.
    pub fn activate(app_id: &str, args: &str) -> Result<(), String> {
        use windows_sys::{
            core::GUID,
            Win32::{
                System::Com::{CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_LOCAL_SERVER, COINIT_APARTMENTTHREADED},
                UI::Shell::ApplicationActivationManager,
            },
        };
        const IID_ACTIVATION_MANAGER: GUID = GUID::from_u128(0x2e941141_7f97_4756_ba1d_9decde894a3d);
        type Activate = unsafe extern "system" fn(*mut std::ffi::c_void, *const u16, *const u16, i32, *mut u32) -> i32;
        type Release = unsafe extern "system" fn(*mut std::ffi::c_void) -> u32;
        let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
        let (id, args) = (wide(app_id), wide(args));
        std::thread::spawn(move || unsafe {
            let com = CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32);
            let mut manager: *mut std::ffi::c_void = std::ptr::null_mut();
            let mut hr = CoCreateInstance(&ApplicationActivationManager, std::ptr::null_mut(), CLSCTX_LOCAL_SERVER, &IID_ACTIVATION_MANAGER, &mut manager);
            if hr >= 0 && !manager.is_null() {
                // IApplicationActivationManager: IUnknown's three, then ActivateApplication.
                let vtable = *(manager as *const *const usize);
                let activate: Activate = std::mem::transmute(*vtable.add(3));
                let release: Release = std::mem::transmute(*vtable.add(2));
                let mut pid = 0u32;
                hr = activate(manager, id.as_ptr(), args.as_ptr(), 0, &mut pid);
                release(manager);
            }
            if com >= 0 {
                CoUninitialize();
            }
            if hr >= 0 { Ok(()) } else { Err(format!("Windows lehnte den Start ab (0x{:08X})", hr as u32)) }
        })
        .join()
        .unwrap_or_else(|_| Err("Start abgebrochen".into()))
    }

    /// Running processes with this exe name.
    pub fn processes(name: &str) -> Vec<Process> {
        let mut out = Vec::new();
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snap == INVALID_HANDLE_VALUE {
                return out;
            }
            let mut e: PROCESSENTRY32W = std::mem::zeroed();
            e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            let mut ok = Process32FirstW(snap, &mut e);
            while ok != 0 {
                let len = e.szExeFile.iter().position(|c| *c == 0).unwrap_or(e.szExeFile.len());
                if String::from_utf16_lossy(&e.szExeFile[..len]).eq_ignore_ascii_case(name) {
                    if let Some(path) = image_path(e.th32ProcessID) {
                        let cmdline = command_line(e.th32ProcessID);
                        out.push(Process { id: e.th32ProcessID, parent: e.th32ParentProcessID, path, cmdline });
                    }
                }
                ok = Process32NextW(snap, &mut e);
            }
            CloseHandle(snap);
        }
        out
    }

    fn image_path(pid: u32) -> Option<String> {
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return None;
            }
            let mut buf = [0u16; 1024];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut len);
            CloseHandle(h);
            (ok != 0).then(|| String::from_utf16_lossy(&buf[..len as usize]))
        }
    }

    /// The process with its start time, which the Restart Manager needs to tell it from a later one with the same id.
    fn unique(pid: u32) -> Option<RM_UNIQUE_PROCESS> {
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return None;
            }
            let (mut created, mut exited, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
            let ok = GetProcessTimes(h, &mut created, &mut exited, &mut kernel, &mut user);
            CloseHandle(h);
            (ok != 0).then_some(RM_UNIQUE_PROCESS { dwProcessId: pid, ProcessStartTime: created })
        }
    }

    /// Asks these apps to quit through the Restart Manager: the "close app" end of
    /// session Windows sends before an update, which apps answer by saving and quitting.
    /// Returns once they quit or declined.
    pub fn request_quit(pids: &[u32]) {
        let apps: Vec<RM_UNIQUE_PROCESS> = pids.iter().filter_map(|id| unique(*id)).collect();
        if apps.is_empty() {
            return;
        }
        unsafe {
            let mut session = 0u32;
            let mut key = [0u16; CCH_RM_SESSION_KEY as usize + 1];
            if RmStartSession(&mut session, 0, key.as_mut_ptr()) != 0 {
                return;
            }
            if RmRegisterResources(session, 0, std::ptr::null(), apps.len() as u32, apps.as_ptr(), 0, std::ptr::null()) == 0 {
                // 0: no forced shutdown; whatever does not quit is ended by the caller.
                RmShutdown(session, 0, None);
            }
            RmEndSession(session);
        }
    }

    /// Ends a process that did not close on its own.
    pub fn end(pid: u32) {
        unsafe {
            let h = OpenProcess(PROCESS_TERMINATE, 0, pid);
            if !h.is_null() {
                TerminateProcess(h, 1);
                CloseHandle(h);
            }
        }
    }
}

#[cfg(not(windows))]
mod win {
    pub struct Process {
        pub id: u32,
        pub parent: u32,
        pub path: String,
        pub cmdline: String,
    }
    pub fn processes(_: &str) -> Vec<Process> {
        Vec::new()
    }
    pub fn activate(_: &str, _: &str) -> Result<(), String> {
        Err("nur unter Windows".into())
    }
    pub fn end(_: u32) {}
    pub fn request_quit(_: &[u32]) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuids() {
        assert!(is_uuid("648f1ecd-7746-44a3-b93d-ca4acfb8f510"));
        assert!(!is_uuid("648f1ecd-7746-44a3-b93d-ca4acfb8f51"));
        assert!(!is_uuid("648F1ECD-7746-44a3-b93d-ca4acfb8f510"));
        assert!(!is_uuid("../../../../../../../../../../../.."));
    }

    #[test]
    fn processes_belong_to_their_data_folder() {
        let p = |id: u32, parent: u32, path: &str, cmdline: &str| win::Process { id, parent, path: path.into(), cmdline: cmdline.into() };
        let app = r"C:\Program Files\WindowsApps\Claude_6_x64__pzs\app\Claude.exe";
        let procs = [
            p(10, 1, app, r#""C:\x\Claude.exe""#),
            p(11, 10, app, r#""C:\x\Claude.exe" --type=gpu-process"#),
            p(20, 1, app, r#""C:\x\Claude.exe" --user-data-dir="C:\Users\a\AppData\Local\ClaudeAccountSwitcher\desktop\B""#),
            p(21, 20, app, r#""C:\x\Claude.exe" --type=renderer"#),
            p(30, 20, r"C:\Users\a\AppData\Local\ClaudeAccountSwitcher\desktop\B\claude-code\2.1\claude.exe", ""),
            p(40, 10, r"C:\Users\a\AppData\Local\Claude-Data\claude_pzs\claude-code\2.1\claude.exe", ""),
        ];
        let by_id: HashMap<u32, &win::Process> = procs.iter().map(|p| (p.id, p)).collect();
        let b = Some(r"c:\users\a\appdata\local\claudeaccountswitcher\desktop\b".to_string());
        let owners: Vec<Option<String>> = procs.iter().map(|p| owner(p, &by_id)).collect();
        assert_eq!(owners[0], None);
        assert_eq!(owners[1], None);
        assert_eq!(owners[2], b);
        assert_eq!(owners[3], b);
        assert_eq!(owners[4], b);
        assert_eq!(owners[5].as_deref(), Some(r"c:\users\a\appdata\local\claude-data\claude_pzs"));
        assert_eq!(data_dir_arg("x --user-data-dir=C:/a/b/ --y").as_deref(), Some(r"c:\a\b"));
    }

    #[test]
    fn family_from_folder() {
        assert_eq!(package_family(Path::new(r"C:\x\Claude-Data\claude_pzs8sxrjxfjjc")), "Claude_pzs8sxrjxfjjc");
    }
    /// Read-only check on a machine with Claude Desktop running: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn finds_running_claude() {
        let ud = find_user_data().expect("Claude Desktop data folder");
        let running = claude_processes(&ud);
        let ids: Vec<u32> = running.iter().map(|(id, _)| *id).collect();
        let apps: Vec<u32> = running.iter().filter(|(_, parent)| !ids.contains(parent)).map(|(id, _)| *id).collect();
        println!("{} -> {} processes, app {:?}, signed in {:?}", ud.display(), ids.len(), apps, signed_in(&ud));
        assert_eq!(apps.len(), 1, "one Claude Desktop main process");
    }
}