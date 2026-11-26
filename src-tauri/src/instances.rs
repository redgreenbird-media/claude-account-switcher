//! Claude Desktop in its own window per account, next to the main one.
//!
//! Claude keeps a `--user-data-dir` it is started with and holds its
//! single-instance lock per data folder, so each account gets a folder of its
//! own, `desktop\<uuid>`, and runs there in parallel to the main Desktop (the
//! default folder, where switching happens) and to the other accounts.
//!
//! The login travels the way a switch moves it: the saved profile is put into
//! the folder before its first start, and whichever place used the account
//! last holds its newest login. The main Desktop takes an account over only
//! after its window is closed, and a window opens only for an account the main
//! Desktop is not signed in with, so no login runs in two places at once.

use crate::{
    claude::{self, read_json, write_json},
    profiles::Store,
};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

/// Settings of the main Desktop a new window starts with.
const CARRIED_KEYS: [&str; 2] = ["locale", "userThemeMode"];

fn modified(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

impl Store {
    pub fn instance(&self, id: &str) -> PathBuf {
        self.root.join("desktop").join(id)
    }

    /// The account the window's folder is signed in with.
    pub fn instance_account(&self, id: &str) -> Option<String> {
        claude::signed_in(&self.instance(id))
    }

    pub fn instance_running(&self, id: &str) -> bool {
        let dir = self.instance(id);
        dir.is_dir() && claude::is_running(&dir)
    }

    /// The saved profile holds a newer login than the window's folder (or the folder has none).
    fn profile_is_newer(&self, id: &str) -> bool {
        let Some(saved) = modified(&self.profiles().join(id).join("auth.json")) else { return false };
        if self.instance_account(id).as_deref() != Some(id) {
            return true;
        }
        modified(&self.instance(id).join("config.json")).is_none_or(|own| saved > own)
    }

    /// Opens `id` in a Claude Desktop window of its own; one already open comes to the front.
    pub fn open_instance(&self, main: &Path, id: &str) -> Result<(), String> {
        if claude::signed_in(main).as_deref() == Some(id) {
            return Err("Dieser Account ist im Haupt-Desktop angemeldet – dort erst zu einem anderen wechseln".into());
        }
        let dir = self.instance(id);
        if !(dir.is_dir() && claude::is_running(&dir)) {
            if self.profile_is_newer(id) {
                self.seed(main, &dir, id)?;
                self.log(&format!("window {id}: login from the saved profile"));
            } else if self.instance_account(id).is_none() && !self.has_profile(id) {
                return Err(format!("kein gespeicherter Login für {id}"));
            }
        }
        self.log(&format!("window {id}: start"));
        claude::start_instance(main, &dir)
    }

    /// Puts the saved login into the window's folder. Its cookies and tokens are encrypted with the
    /// key in the main Desktop's `Local State`, so the folder gets that key too.
    fn seed(&self, main: &Path, dir: &Path, id: &str) -> Result<(), String> {
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let main_state = read_json(&main.join("Local State")).ok_or("Local State des Haupt-Desktops unlesbar")?;
        let key = main_state.get("os_crypt").cloned().ok_or("Local State ohne Schlüssel")?;
        let state_path = dir.join("Local State");
        let mut state = read_json(&state_path).filter(Value::is_object).unwrap_or_else(|| json!({}));
        if state.get("os_crypt") != Some(&key) {
            state["os_crypt"] = key;
            write_json(&state_path, &state)?;
        }
        let config_path = dir.join("config.json");
        if read_json(&config_path).is_none() {
            let from = read_json(&main.join("config.json")).unwrap_or(Value::Null);
            let mut config = serde_json::Map::new();
            for k in CARRIED_KEYS {
                if let Some(v) = from.get(k) {
                    config.insert(k.into(), v.clone());
                }
            }
            write_json(&config_path, &Value::Object(config))?;
        }
        self.restore(dir, id)
    }

    /// Before the main Desktop takes `id` over: closes the account's window and keeps its login
    /// if it is newer than the saved one.
    pub fn take_back(&self, id: &str) -> Result<(), String> {
        let dir = self.instance(id);
        if !dir.is_dir() {
            return Ok(());
        }
        if claude::is_running(&dir) {
            claude::stop(&dir)?;
            self.log(&format!("window {id}: closed for the switch"));
        }
        if self.instance_account(id).as_deref() == Some(id) && !self.profile_is_newer(id) {
            self.save(&dir, id)?;
            self.log(&format!("window {id}: login saved"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profiles::Target;

    const A: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    const B: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";

    fn sign_in(user_data: &Path, id: &str, cookie: &str) {
        fs::create_dir_all(user_data.join("Network")).unwrap();
        fs::write(user_data.join("Network\\Cookies"), cookie).unwrap();
        let mut config = read_json(&user_data.join("config.json")).unwrap_or(json!({ "locale": "de-CH" }));
        config["oauth:tokenCacheV2"] = Value::String(format!("token-{cookie}"));
        config["lastKnownAccountUuid"] = Value::String(id.into());
        write_json(&user_data.join("config.json"), &config).unwrap();
    }

    #[test]
    fn window_gets_saved_login_and_gives_newer_one_back() {
        let root = std::env::temp_dir().join(format!("claude-accounts-test-instances-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let main = root.join("main");
        let store = Store { root: root.join("store") };
        sign_in(&main, A, "cookie-a");
        write_json(&main.join("Local State"), &json!({ "os_crypt": { "encrypted_key": "KEY" } })).unwrap();
        // A is saved and B signed in, as after a switch.
        store.swap(&main, Some(A), &Target::New, false).unwrap();
        sign_in(&main, B, "cookie-b");

        let dir = store.instance(A);
        assert!(store.profile_is_newer(A));
        store.seed(&main, &dir, A).unwrap();
        assert_eq!(claude::signed_in(&dir).as_deref(), Some(A));
        assert_eq!(fs::read_to_string(dir.join("Network\\Cookies")).unwrap(), "cookie-a");
        assert_eq!(read_json(&dir.join("Local State")).unwrap()["os_crypt"]["encrypted_key"], "KEY");
        assert_eq!(read_json(&dir.join("config.json")).unwrap()["locale"], "de-CH");
        assert!(!store.profile_is_newer(A));

        // The window renewed its login; the main Desktop takes it back.
        std::thread::sleep(std::time::Duration::from_millis(50));
        sign_in(&dir, A, "cookie-a2");
        store.take_back(A).unwrap();
        store.swap(&main, Some(B), &Target::Account(A.into()), false).unwrap();
        assert_eq!(fs::read_to_string(main.join("Network\\Cookies")).unwrap(), "cookie-a2");
        let _ = fs::remove_dir_all(root);
    }
}
