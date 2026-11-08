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

/// Claude Desktop's own processes and its Code sessions' engines.
fn claude_pids(user_data: &Path) -> Vec<u32> {
    let engine = user_data.join("claude-code").to_string_lossy().to_ascii_lowercase();
    win::processes("claude.exe")
        .into_iter()
        .filter(|(_, p)| {
            let p = p.to_ascii_lowercase();
            p.contains("\\windowsapps\\claude_") || p.starts_with(&engine)
        })
        .map(|(id, _)| id)
        .collect()
}

/// Closes Claude Desktop: its windows first, then whatever is left.
pub fn stop(user_data: &Path) -> Result<(), String> {
    let running = claude_pids(user_data);
    if running.is_empty() {
        return Ok(());
    }
    win::close_windows(&running);
    if !wait_gone(user_data, Duration::from_secs(5)) {
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
        Foundation::{CloseHandle, HWND, INVALID_HANDLE_VALUE, LPARAM},
        System::{
            Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS},
            Threading::{OpenProcess, QueryFullProcessImageNameW, TerminateProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE},
        },
        UI::WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, WM_CLOSE},
    };

    /// Running processes with this exe name: (pid, full path).
    pub fn processes(name: &str) -> Vec<(u32, String)> {
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
                        out.push((e.th32ProcessID, path));
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

    /// Asks every visible top-level window of these processes to close.
    pub fn close_windows(pids: &[u32]) {
        unsafe extern "system" fn each(hwnd: HWND, data: LPARAM) -> windows_sys::core::BOOL {
            let pids = &*(data as *const Vec<u32>);
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, &mut pid);
            if pids.contains(&pid) && IsWindowVisible(hwnd) != 0 {
                PostMessageW(hwnd, WM_CLOSE, 0, 0);
            }
            1
        }
        let list = pids.to_vec();
        unsafe {
            EnumWindows(Some(each), &list as *const Vec<u32> as LPARAM);
        }
    }
}

#[cfg(not(windows))]
mod win {
    pub fn processes(_: &str) -> Vec<(u32, String)> {
        Vec::new()
    }
    pub fn end(_: u32) {}
    pub fn close_windows(_: &[u32]) {}
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
        let pids = claude_pids(&ud);
        println!("{} -> {} processes, signed in {:?}", ud.display(), pids.len(), signed_in(&ud));
        assert!(!pids.is_empty());
    }
}