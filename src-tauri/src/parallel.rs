//! Parallel Claude Code sessions with another account, in a terminal.
//!
//! Claude Desktop runs one account at a time (signed builds ignore
//! `CLAUDE_USER_DATA_DIR`, the only way to a second instance). The Claude Code
//! CLI keeps its login in `CLAUDE_CONFIG_DIR`, so each account gets its own
//! config folder `~/.claude-accounts/<uuid>`: signed in once with `/login`,
//! it runs next to Desktop and next to the other accounts.
//!
//! Skills, agents, commands, plugins and projects (sessions, memory) are the
//! ones of `~/.claude` (junctions); settings, the CLAUDE.md files and the MCP
//! servers are copied over at every start.

use crate::claude::read_json;
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

const SHARED_DIRS: [&str; 7] = ["skills", "agents", "commands", "plugins", "projects", "output-styles", "hooks"];
const SHARED_KEYS: [&str; 1] = ["mcpServers"];
const FIRST_RUN_KEYS: [&str; 2] = ["theme", "hasCompletedOnboarding"];
/// What a Claude Code or Desktop session hands its children: its own login and session. Started from
/// such a terminal, the CLI would sign in as that session's account instead of this folder's.
const INHERITED_AUTH: [&str; 18] = [
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "CLAUDECODE",
    "CLAUDE_CODE_ACCOUNT_UUID",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_EXECPATH",
    "CLAUDE_CODE_HOST_SESSION_ID",
    "CLAUDE_CODE_MESSAGING_SOCKET",
    "CLAUDE_CODE_MESSAGING_TOKEN",
    "CLAUDE_CODE_OAUTH_REFRESH_TOKEN",
    "CLAUDE_CODE_OAUTH_SCOPES",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "CLAUDE_CODE_ORGANIZATION_UUID",
    "CLAUDE_CODE_SDK_HAS_HOST_AUTH_REFRESH",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_USER_EMAIL",
    "CLAUDE_CONFIG_DIR",
];

fn home() -> PathBuf {
    std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_default()
}

fn main_config() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from).unwrap_or_else(|| home().join(".claude"))
}

/// `~/.claude.json` of the main config: next to `.claude` unless CLAUDE_CONFIG_DIR moves it.
fn main_global() -> PathBuf {
    match std::env::var_os("CLAUDE_CONFIG_DIR") {
        Some(d) => PathBuf::from(d).join(".claude.json"),
        None => home().join(".claude.json"),
    }
}

pub fn dir(id: &str) -> PathBuf {
    home().join(".claude-accounts").join(id)
}

/// The e-mail the account's CLI login belongs to, once `/login` ran.
pub fn signed_in_as(id: &str) -> Option<String> {
    let v = read_json(&dir(id).join(".claude.json"))?;
    v.get("oauthAccount")?.get("emailAddress")?.as_str().map(str::to_string)
}

fn hidden(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

fn junction(link: &Path, target: &Path) -> Result<(), String> {
    let ok = hidden(Command::new("cmd.exe").arg("/c").arg("mklink").arg("/J").arg(link).arg(target))
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if ok { Ok(()) } else { Err(format!("Verknüpfung {} -> {} fehlgeschlagen", link.display(), target.display())) }
}

/// Creates or refreshes the account's config folder.
pub fn prepare(id: &str) -> Result<PathBuf, String> {
    let main = main_config();
    let own = dir(id);
    fs::create_dir_all(&own).map_err(|e| format!("{}: {e}", own.display()))?;

    for name in SHARED_DIRS {
        let (src, dst) = (main.join(name), own.join(name));
        if src.is_dir() && fs::symlink_metadata(&dst).is_err() {
            junction(&dst, &src)?;
        }
    }
    for entry in fs::read_dir(&main).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let lower = name.to_ascii_lowercase();
        if entry.path().is_file() && (lower == "settings.json" || lower.ends_with(".md")) {
            fs::copy(entry.path(), own.join(&name)).map_err(|e| format!("{name}: {e}"))?;
        }
    }

    let from = read_json(&main_global()).unwrap_or(Value::Null);
    let path = own.join(".claude.json");
    let mut mine = read_json(&path).unwrap_or_else(|| serde_json::json!({}));
    if let (Some(src), Some(dst)) = (from.as_object(), mine.as_object_mut()) {
        for key in SHARED_KEYS {
            match src.get(key) {
                Some(v) => dst.insert(key.into(), v.clone()),
                None => dst.remove(key),
            };
        }
        for key in FIRST_RUN_KEYS {
            if let (Some(v), false) = (src.get(key), dst.contains_key(key)) {
                dst.insert(key.into(), v.clone());
            }
        }
    }
    crate::claude::write_json(&path, &mine)?;
    Ok(own)
}

fn on_path(exe: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|p| std::env::split_paths(&p).map(|d| d.join(exe)).find(|f| f.is_file()))
}

/// Quotes one argument for a Windows command line.
fn quote(s: &str) -> String {
    let s = s.trim_end_matches('\\');
    // "C:\" would end in an escaped quote; "C:\." names the same folder.
    let s = if s.ends_with(':') { format!("{s}\\.") } else { s.to_string() };
    format!("\"{}\"", s.replace('"', ""))
}

/// Opens a terminal in `cwd` running Claude Code with the account's own login.
pub fn launch(id: &str, label: &str, cwd: &Path) -> Result<(), String> {
    if !cwd.is_dir() {
        return Err(format!("Ordner {} gibt es nicht", cwd.display()));
    }
    let claude = on_path("claude.exe").ok_or("Claude Code (claude.exe) nicht gefunden – im Terminal installieren: irm https://claude.ai/install.ps1 | iex")?;
    let config = prepare(id)?;
    // A start file, so no shell has to parse `&&` on the way: Windows Terminal may hand the tab
    // to a window it already runs, which would not inherit our environment.
    let script = config.join("start-claude.cmd");
    let clear: String = INHERITED_AUTH.iter().map(|name| format!("set \"{name}=\"\r\n")).collect();
    let body = format!(
        "@echo off\r\ntitle Claude - {label}\r\n{clear}set \"CLAUDE_CONFIG_DIR={}\"\r\n{}\r\n",
        config.display(),
        quote(&claude.to_string_lossy())
    );
    fs::write(&script, body).map_err(|e| format!("{}: {e}", script.display()))?;
    let script = quote(&script.to_string_lossy());
    let dir = quote(&cwd.to_string_lossy());

    #[cfg(windows)]
    {
        if on_path("wt.exe").is_some() {
            let title = quote(&format!("Claude - {label}"));
            return win::shell("wt.exe", &format!("-w new new-tab --title {title} --suppressApplicationTitle -d {dir} cmd.exe /k {script}"), cwd);
        }
        return win::shell("cmd.exe", &format!("/k {script}"), cwd);
    }
    #[allow(unreachable_code)]
    Err(format!("{script} {dir}: nur unter Windows"))
}

#[cfg(windows)]
mod win {
    use std::{os::windows::ffi::OsStrExt, path::Path};
    use windows_sys::Win32::{
        System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE},
        UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
    };

    fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
        s.encode_wide().chain(Some(0)).collect()
    }

    /// Starts `file` the way Explorer does: app execution aliases such as wt.exe only start this way.
    /// On a thread of its own with a single-threaded COM apartment, which ShellExecute needs for
    /// packaged apps (from Tauri's threads it answers "access denied").
    pub fn shell(file: &str, params: &str, cwd: &Path) -> Result<(), String> {
        let (f, p, d) = (wide(file.as_ref()), wide(params.as_ref()), wide(cwd.as_os_str()));
        let r = std::thread::spawn(move || unsafe {
            let com = CoInitializeEx(std::ptr::null(), (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32);
            let op = wide("open".as_ref());
            let r = ShellExecuteW(std::ptr::null_mut(), op.as_ptr(), f.as_ptr(), p.as_ptr(), d.as_ptr(), SW_SHOWNORMAL) as isize;
            if com >= 0 {
                CoUninitialize();
            }
            r
        })
        .join()
        .unwrap_or(0);
        if r > 32 { Ok(()) } else { Err(format!("{file} starten fehlgeschlagen ({r})")) }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_paths() {
        assert_eq!(quote(r"C:\Users\a b"), r#""C:\Users\a b""#);
        assert_eq!(quote(r"C:\"), r#""C:\.""#);
        assert_eq!(quote(r"D:\x\"), r#""D:\x""#);
    }
    #[test]
    fn prepares_config_folder_from_main_one() {
        let home = std::env::temp_dir().join(format!("claude-accounts-home-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        let main = home.join(".claude");
        fs::create_dir_all(main.join("skills").join("x")).unwrap();
        fs::write(main.join("skills").join("x").join("SKILL.md"), "skill").unwrap();
        fs::write(main.join("settings.json"), "{\"model\":\"opus\"}").unwrap();
        fs::write(main.join("CLAUDE.md"), "rules").unwrap();
        fs::write(main.join("history.jsonl"), "private").unwrap();
        fs::write(home.join(".claude.json"), r#"{"mcpServers":{"a":{}},"theme":"dark","oauthAccount":{"emailAddress":"main@x.ch"}}"#).unwrap();
        std::env::set_var("USERPROFILE", &home);
        std::env::remove_var("CLAUDE_CONFIG_DIR");

        let id = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
        let own = prepare(id).unwrap();
        assert_eq!(own, home.join(".claude-accounts").join(id));
        assert_eq!(fs::read_to_string(own.join("skills").join("x").join("SKILL.md")).unwrap(), "skill");
        assert_eq!(fs::read_to_string(own.join("CLAUDE.md")).unwrap(), "rules");
        assert!(own.join("settings.json").is_file());
        assert!(!own.join("history.jsonl").exists());
        let global = read_json(&own.join(".claude.json")).unwrap();
        assert!(global["mcpServers"]["a"].is_object());
        assert_eq!(global["theme"], "dark");
        // The main login never travels: the folder signs in on its own.
        assert!(global.get("oauthAccount").is_none());
        assert_eq!(signed_in_as(id), None);

        // A second start keeps the junction and the folder's own login.
        let mut mine = global.clone();
        mine["oauthAccount"] = serde_json::json!({ "emailAddress": "b@x.ch" });
        crate::claude::write_json(&own.join(".claude.json"), &mine).unwrap();
        prepare(id).unwrap();
        assert_eq!(signed_in_as(id).as_deref(), Some("b@x.ch"));
        let _ = fs::remove_dir_all(&home);
    }
}