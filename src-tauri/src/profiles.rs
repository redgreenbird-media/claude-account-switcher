//! Swapping Claude Desktop's login: save the signed-in account's login into
//! `profiles\<uuid>`, put another saved one back (or none, for a new sign-in).
//!
//! Only login data moves: the session cookies, the web storage of the
//! claude.ai views and two keys of config.json. Sessions and settings stay.

use crate::claude::{self, is_uuid, read_json, write_json};
use serde_json::{Map, Value};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub const ITEMS: [&str; 5] = ["Network\\Cookies", "Network\\Cookies-journal", "Local Storage", "Session Storage", "IndexedDB"];
const AUTH_KEYS: [&str; 2] = ["oauth:tokenCacheV2", "lastKnownAccountUuid"];

pub enum Target {
    Account(String),
    /// Signed out, so Claude shows its sign-in page.
    New,
}

pub struct Store {
    pub root: PathBuf,
}

impl Store {
    pub fn profiles(&self) -> PathBuf {
        self.root.join("profiles")
    }

    pub fn has_profile(&self, id: &str) -> bool {
        is_uuid(id) && self.profiles().join(id).join("auth.json").is_file()
    }

    /// Saved accounts, by uuid.
    pub fn saved(&self) -> Vec<String> {
        let mut ids: Vec<String> = fs::read_dir(self.profiles())
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|id| self.has_profile(id))
            .collect();
        ids.sort();
        ids
    }

    pub fn log(&self, line: &str) {
        let _ = fs::create_dir_all(&self.root);
        if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(self.root.join("switch.log")) {
            let now = crate::usage::now_ms();
            let _ = writeln!(f, "{}  {line}", crate::usage::stamp(now));
        }
    }

    /// Closes Claude, saves the current login, puts `target` in place and starts Claude again.
    /// `expect` is the account the caller believes is signed in; any other stops the swap unchanged.
    pub fn swap(&self, user_data: &Path, expect: Option<&str>, target: &Target, control_app: bool) -> Result<(), String> {
        if let Target::Account(id) = target {
            if !self.has_profile(id) {
                return Err(format!("kein gespeicherter Login für {id}"));
            }
        }
        if !user_data.join("config.json").is_file() {
            return Err(format!("keine config.json in {}", user_data.display()));
        }
        if control_app {
            claude::stop(user_data)?;
        }
        let result = self.swap_files(user_data, expect, target);
        if let Err(e) = &result {
            self.log(&format!("error: {e}"));
        }
        if control_app {
            if let Err(e) = claude::start(user_data) {
                self.log(&format!("start failed: {e}"));
            }
        }
        result
    }

    fn swap_files(&self, user_data: &Path, expect: Option<&str>, target: &Target) -> Result<(), String> {
        let current = claude::signed_in(user_data);
        if let (Some(cur), Some(exp)) = (&current, expect) {
            if cur != exp {
                return Err(format!("angemeldet ist {cur}, erwartet {exp}; nichts geändert"));
            }
        }
        if let Some(cur) = &current {
            self.save(user_data, cur)?;
            self.log(&format!("saved {cur}"));
        }
        let put = match target {
            Target::Account(id) => self.restore(user_data, id).map(|_| self.log(&format!("restored {id}"))),
            Target::New => clear(user_data).map(|_| self.log("cleared login for a new account")),
        };
        if let Err(e) = put {
            self.log(&format!("restore failed: {e}; putting {} back", current.as_deref().unwrap_or("-")));
            if let Some(cur) = &current {
                self.restore(user_data, cur)?;
            }
            return Err(e);
        }
        Ok(())
    }

    /// Snapshot of the login into `profiles\<id>`, swapped in only once complete.
    pub fn save(&self, user_data: &Path, id: &str) -> Result<(), String> {
        if !is_uuid(id) {
            return Err(format!("ungültige Account-ID {id}"));
        }
        let dir = self.profiles().join(id);
        let new = self.profiles().join(format!("{id}.new"));
        let old = self.profiles().join(format!("{id}.old"));
        remove(&new)?;
        fs::create_dir_all(&new).map_err(|e| e.to_string())?;
        for item in ITEMS {
            copy_replace(&user_data.join(item), &new.join("data").join(item))?;
        }
        let config = read_json(&user_data.join("config.json")).ok_or("config.json unlesbar")?;
        let mut auth = Map::new();
        for key in AUTH_KEYS {
            if let Some(v) = config.get(key) {
                auth.insert(key.into(), v.clone());
            }
        }
        write_json(&new.join("auth.json"), &Value::Object(auth))?;
        remove(&old)?;
        if dir.exists() {
            fs::rename(&dir, &old).map_err(|e| e.to_string())?;
        }
        fs::rename(&new, &dir).map_err(|e| e.to_string())?;
        remove(&old)
    }

    fn restore(&self, user_data: &Path, id: &str) -> Result<(), String> {
        let dir = self.profiles().join(id);
        let auth = read_json(&dir.join("auth.json")).ok_or_else(|| format!("kein gespeicherter Login für {id}"))?;
        for item in ITEMS {
            copy_replace(&dir.join("data").join(item), &user_data.join(item))?;
        }
        set_auth(user_data, Some(&auth))
    }
}

fn clear(user_data: &Path) -> Result<(), String> {
    for item in ITEMS {
        remove(&user_data.join(item))?;
    }
    set_auth(user_data, None)
}

fn set_auth(user_data: &Path, auth: Option<&Value>) -> Result<(), String> {
    let path = user_data.join("config.json");
    let mut config = read_json(&path).ok_or("config.json unlesbar")?;
    let obj = config.as_object_mut().ok_or("config.json ist kein Objekt")?;
    for key in AUTH_KEYS {
        match auth.and_then(|a| a.get(key)) {
            Some(v) => {
                obj.insert(key.into(), v.clone());
            }
            None => {
                obj.remove(key);
            }
        }
    }
    write_json(&path, &config)
}

fn remove(path: &Path) -> Result<(), String> {
    let r = match fs::symlink_metadata(path) {
        Err(_) => return Ok(()),
        Ok(m) if m.is_dir() => fs::remove_dir_all(path),
        Ok(_) => fs::remove_file(path),
    };
    r.map_err(|e| format!("{} löschen: {e}", path.display()))
}

/// Replaces `dst` with a copy of `src`; a missing source leaves no destination.
fn copy_replace(src: &Path, dst: &Path) -> Result<(), String> {
    remove(dst)?;
    let Ok(meta) = fs::metadata(src) else { return Ok(()) };
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    if meta.is_dir() {
        copy_tree(src, dst)
    } else {
        copy_file(src, dst)
    }
}

fn copy_tree(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(src).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let to = dst.join(entry.file_name());
        if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            copy_tree(&entry.path(), &to)?;
        } else {
            copy_file(&entry.path(), &to)?;
        }
    }
    Ok(())
}

fn copy_file(src: &Path, dst: &Path) -> Result<(), String> {
    // LevelDB's LOCK files can stay held a moment after Claude exits; they carry no data.
    match fs::copy(src, dst) {
        Ok(_) => Ok(()),
        Err(_) if src.file_name().is_some_and(|n| n == "LOCK") => Ok(()),
        Err(e) => Err(format!("{} kopieren: {e}", src.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    const B: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("claude-accounts-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sign_in(user_data: &Path, id: &str, cookie: &str) {
        fs::create_dir_all(user_data.join("Network")).unwrap();
        fs::write(user_data.join("Network\\Cookies"), cookie).unwrap();
        fs::create_dir_all(user_data.join("Local Storage\\leveldb")).unwrap();
        fs::write(user_data.join("Local Storage\\leveldb\\000003.log"), cookie).unwrap();
        let mut config = read_json(&user_data.join("config.json")).unwrap_or(serde_json::json!({ "locale": "de-CH" }));
        config["oauth:tokenCacheV2"] = Value::String(format!("token-{cookie}"));
        config["lastKnownAccountUuid"] = Value::String(id.into());
        write_json(&user_data.join("config.json"), &config).unwrap();
    }

    #[test]
    fn switch_round_trip_keeps_other_settings() {
        let root = temp("roundtrip");
        let ud = root.join("ud");
        let store = Store { root: root.join("store") };
        sign_in(&ud, A, "cookie-a");
        store.swap(&ud, Some(A), &Target::New, false).unwrap();
        assert_eq!(claude::signed_in(&ud), None);
        assert!(!ud.join("Network\\Cookies").exists());
        assert_eq!(read_json(&ud.join("config.json")).unwrap()["locale"], "de-CH");

        sign_in(&ud, B, "cookie-b");
        store.swap(&ud, Some(B), &Target::Account(A.into()), false).unwrap();
        assert_eq!(claude::signed_in(&ud).as_deref(), Some(A));
        assert_eq!(fs::read_to_string(ud.join("Network\\Cookies")).unwrap(), "cookie-a");
        assert_eq!(fs::read_to_string(ud.join("Local Storage\\leveldb\\000003.log")).unwrap(), "cookie-a");
        assert_eq!(store.saved(), vec![A.to_string(), B.to_string()]);

        store.swap(&ud, Some(A), &Target::Account(B.into()), false).unwrap();
        assert_eq!(claude::signed_in(&ud).as_deref(), Some(B));
        assert_eq!(fs::read_to_string(ud.join("Network\\Cookies")).unwrap(), "cookie-b");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn refuses_when_another_account_is_signed_in() {
        let root = temp("mismatch");
        let ud = root.join("ud");
        let store = Store { root: root.join("store") };
        sign_in(&ud, A, "cookie-a");
        store.save(&ud, A).unwrap();
        sign_in(&ud, B, "cookie-b");
        let err = store.swap(&ud, Some(A), &Target::Account(A.into()), false).unwrap_err();
        assert!(err.contains("nichts geändert"), "{err}");
        assert_eq!(claude::signed_in(&ud).as_deref(), Some(B));
        assert_eq!(fs::read_to_string(ud.join("Network\\Cookies")).unwrap(), "cookie-b");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn unknown_target_changes_nothing() {
        let root = temp("unknown");
        let ud = root.join("ud");
        let store = Store { root: root.join("store") };
        sign_in(&ud, A, "cookie-a");
        assert!(store.swap(&ud, None, &Target::Account(B.into()), false).is_err());
        assert_eq!(claude::signed_in(&ud).as_deref(), Some(A));
        let _ = fs::remove_dir_all(root);
    }
}