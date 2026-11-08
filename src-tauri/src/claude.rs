//! Claude Desktop on disk and as a process: its data folder, the signed-in
//! account, closing and starting it.

use serde_json::Value;
use std::{
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

/// Claude Desktop's own processes and its Code sessions' engines: (pid, parent pid).
fn claude_processes(user_data: &Path) -> Vec<(u32, u32)> {
    let engine = user_data.join("claude-code").to_string_lossy().to_ascii_lowercase();
    win::processes("claude.exe")
        .into_iter()
        .filter(|p| {
            let path = p.path.to_ascii_lowercase();
            path.contains("\\windowsapps\\claude_") || path.starts_with(&engine)
        })
        .map(|p| (p.id, p.parent))
        .collect()
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
                        out.push(Process { id: e.th32ProcessID, parent: e.th32ParentProcessID, path });
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
    }
    pub fn processes(_: &str) -> Vec<Process> {
        Vec::new()
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