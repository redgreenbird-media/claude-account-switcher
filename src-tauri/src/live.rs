//! Live limits from Anthropic: the usage endpoint Claude Desktop asks itself
//! (`/api/oauth/usage`), with each account's own token from the logins Claude
//! keeps: the main Desktop's, each account's own window's and the saved profiles'.
//!
//! Read only. `oauth:tokenCacheV2` is decrypted in memory with the key in
//! `Local State` (Chromium's os_crypt: an AES-256-GCM key wrapped with DPAPI,
//! bound to the Windows user). Tokens are never written, refreshed or sent
//! anywhere but to Anthropic's API host. An expired token is skipped until
//! Claude renews it; the account then shows its last reading.
//!
//! Each reading goes to `limits\<uuid>.json`, the file the mod writes after a
//! reply, so the app, the mod's band and the next start all see it.

use crate::{
    claude::{read_json, write_json},
    usage::{now_ms, KINDS},
};
use aes_gcm::{aead::Aead, Aes256Gcm, KeyInit, Nonce};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    hash::{DefaultHasher, Hash, Hasher},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

const CACHE_KEY: &str = "oauth:tokenCacheV2";
const ACCOUNT_PREFIX: &str = "acct:";
/// A token this close to its expiry is left for Claude to renew.
const MARGIN_MS: i64 = 60_000;

/// One access token from a login, with what its cache key says about it.
#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub account: String,
    pub token: String,
    pub expires_at: i64,
    /// `https://api.anthropic.com`
    pub host: String,
    pub scope: String,
}

impl Token {
    fn fingerprint(&self) -> u64 {
        let mut h = DefaultHasher::new();
        self.token.hash(&mut h);
        h.finish()
    }
}

/// A login on disk: the JSON holding `oauth:tokenCacheV2` and the `Local State` with its key.
pub struct Source {
    pub config: PathBuf,
    pub local_state: PathBuf,
}

/// The token cache's entries: `acct:<uuid>|<client>:<…>:<api host>:<scope>` -> `{ token, expiresAt, … }`.
pub fn parse_cache(cache: &Value) -> Vec<Token> {
    let Some(entries) = cache.as_object() else { return Vec::new() };
    entries
        .iter()
        .filter_map(|(key, entry)| {
            let rest = key.strip_prefix(ACCOUNT_PREFIX)?;
            let (account, rest) = rest.split_once('|')?;
            // Host and scopes both hold colons (`https://…`, `user:profile`): the host runs from
            // `https://` to the next colon, the scopes are the rest.
            let at = rest.find(":https://")? + 1;
            let after = &rest[at..];
            let end = "https://".len() + after["https://".len()..].find(':')?;
            let (host, scope) = (&after[..end], &after[end + 1..]);
            Some(Token {
                account: account.to_ascii_lowercase(),
                token: entry.get("token")?.as_str()?.to_string(),
                expires_at: entry.get("expiresAt")?.as_f64()? as i64,
                host: host.trim_end_matches('/').to_string(),
                scope: scope.to_string(),
            })
        })
        .filter(|t| crate::claude::is_uuid(&t.account) && allowed_host(&t.host))
        .collect()
}

/// Tokens only ever go to Anthropic's own API over https.
fn allowed_host(host: &str) -> bool {
    let Some(name) = host.strip_prefix("https://") else { return false };
    !name.contains(['/', '@', '?', '#']) && (name == "api.anthropic.com" || name.ends_with(".anthropic.com"))
}

fn os_crypt_key(local_state: &Path) -> Option<Vec<u8>> {
    let state = read_json(local_state)?;
    let wrapped = STANDARD.decode(state.get("os_crypt")?.get("encrypted_key")?.as_str()?).ok()?;
    win::unprotect(wrapped.strip_prefix(b"DPAPI")?)
}

/// Chromium's `v10` format: nonce (12 bytes), then ciphertext with its tag.
fn decrypt(key: &[u8], data: &[u8]) -> Option<Vec<u8>> {
    let body = data.strip_prefix(b"v10")?;
    if body.len() < 12 + 16 {
        return None;
    }
    let (nonce, sealed) = body.split_at(12);
    Aes256Gcm::new_from_slice(key).ok()?.decrypt(Nonce::from_slice(nonce), sealed).ok()
}

fn tokens_of(source: &Source, keys: &mut HashMap<PathBuf, Option<Vec<u8>>>) -> Vec<Token> {
    let Some(sealed) = read_json(&source.config).and_then(|c| c.get(CACHE_KEY)?.as_str().map(str::to_string)) else { return Vec::new() };
    let key = keys.entry(source.local_state.clone()).or_insert_with(|| os_crypt_key(&source.local_state));
    let Some(key) = key.as_deref() else { return Vec::new() };
    let Some(plain) = STANDARD.decode(sealed).ok().and_then(|d| decrypt(key, &d)) else { return Vec::new() };
    serde_json::from_slice(&plain).map(|v: Value| parse_cache(&v)).unwrap_or_default()
}

/// The usage figures the endpoint answers: kind -> (percent, reset as RFC 3339).
fn parse_usage(body: &Value) -> Vec<(String, f64, Option<String>)> {
    KINDS
        .iter()
        .filter_map(|(kind, _, _)| {
            let w = body.get(*kind)?;
            let percent = w.get("utilization")?.as_f64()?;
            let reset = w.get("resets_at").and_then(Value::as_str).map(str::to_string);
            Some((kind.to_string(), percent, reset))
        })
        .collect()
}

pub enum Fetched {
    Usage(Vec<(String, f64, Option<String>)>),
    /// The server refused the token (401/403): not tried again.
    Refused,
    Failed,
}

fn fetch(agent: &ureq::Agent, t: &Token) -> Fetched {
    let response = agent
        .get(&format!("{}/api/oauth/usage", t.host))
        .set("Authorization", &format!("Bearer {}", t.token))
        .set("anthropic-beta", "oauth-2025-04-20")
        .call();
    match response {
        Ok(r) => match r.into_json::<Value>() {
            Ok(body) => Fetched::Usage(parse_usage(&body)),
            Err(_) => Fetched::Failed,
        },
        Err(ureq::Error::Status(401 | 403, _)) => Fetched::Refused,
        Err(_) => Fetched::Failed,
    }
}

/// Polls Anthropic for every account a login is known for. Keeps the refused tokens in memory.
pub struct Poller {
    agent: ureq::Agent,
    refused: HashSet<u64>,
}

impl Poller {
    pub fn new() -> Option<Self> {
        let tls = native_tls::TlsConnector::new().ok()?;
        let agent = ureq::AgentBuilder::new().tls_connector(Arc::new(tls)).timeout(Duration::from_secs(15)).user_agent("claude-accounts").build();
        Some(Poller { agent, refused: HashSet::new() })
    }

    /// Reads each account's limits and writes them to `limits\<uuid>.json`. Returns the accounts read.
    pub fn poll(&mut self, sources: &[Source], limits: &Path) -> Vec<String> {
        let now = now_ms();
        let mut keys = HashMap::new();
        let mut by_account: HashMap<String, Vec<Token>> = HashMap::new();
        for source in sources {
            for t in tokens_of(source, &mut keys) {
                if t.expires_at > now + MARGIN_MS && !self.refused.contains(&t.fingerprint()) {
                    by_account.entry(t.account.clone()).or_default().push(t);
                }
            }
        }
        let mut read = Vec::new();
        for (account, mut tokens) in by_account {
            // The longest-lived first; a token for the profile scope before the others.
            tokens.sort_by_key(|t| (!t.scope.split(' ').any(|s| s == "user:profile"), -t.expires_at));
            tokens.dedup_by(|a, b| a.token == b.token);
            for t in tokens.iter().take(3) {
                match fetch(&self.agent, t) {
                    Fetched::Usage(windows) if !windows.is_empty() => {
                        if record(limits, &account, &windows, now).is_ok() {
                            read.push(account.clone());
                        }
                        break;
                    }
                    Fetched::Refused => {
                        self.refused.insert(t.fingerprint());
                    }
                    _ => {}
                }
            }
        }
        read
    }
}

/// Writes the reading in the mod's format, keeping the e-mail and org the mod recorded.
fn record(limits: &Path, account: &str, windows: &[(String, f64, Option<String>)], now: i64) -> Result<(), String> {
    let path = limits.join(format!("{account}.json"));
    let old = read_json(&path).unwrap_or(Value::Null);
    let keep = |k: &str| old.get(k).cloned().unwrap_or(Value::Null);
    let file = json!({
        "id": account,
        "email": keep("email"),
        "orgId": keep("orgId"),
        "limits": windows.iter().map(|(kind, percent, reset)| json!({ "kind": kind, "percentUsed": percent, "resetsAt": reset })).collect::<Vec<_>>(),
        "measuredAt": now,
        "source": "live",
    });
    std::fs::create_dir_all(limits).map_err(|e| e.to_string())?;
    write_json(&path, &file)
}

#[cfg(windows)]
mod win {
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{CryptUnprotectData, CRYPT_INTEGER_BLOB},
    };

    /// DPAPI: unwraps data the current Windows user wrapped.
    pub fn unprotect(data: &[u8]) -> Option<Vec<u8>> {
        unsafe {
            let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
            let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
            if CryptUnprotectData(&input, std::ptr::null_mut(), std::ptr::null(), std::ptr::null(), std::ptr::null(), 0, &mut out) == 0 {
                return None;
            }
            let plain = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
            LocalFree(out.pbData.cast());
            Some(plain)
        }
    }
}

#[cfg(not(windows))]
mod win {
    pub fn unprotect(_: &[u8]) -> Option<Vec<u8>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "648f1ecd-7746-44a3-b93d-ca4acfb8f510";

    #[test]
    fn reads_cache_keys_and_only_anthropic_hosts() {
        let cache = json!({
            format!("acct:{A}|client-1:org-1:https://api.anthropic.com:user:profile user:inference"): { "token": "t1", "refreshToken": "r", "expiresAt": 2000.0 },
            format!("acct:{A}|client-1:org-1:https://evil.example.com:user:profile"): { "token": "t2", "refreshToken": "r", "expiresAt": 2000.0 },
            "client-1:org-1:https://api.anthropic.com:user:profile": { "token": "t3", "refreshToken": "r", "expiresAt": 2000.0 },
        });
        let tokens = parse_cache(&cache);
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].account, A);
        assert_eq!(tokens[0].host, "https://api.anthropic.com");
        assert_eq!(tokens[0].scope, "user:profile user:inference");
        assert!(!allowed_host("https://api.anthropic.com.evil.io"));
        assert!(!allowed_host("http://api.anthropic.com"));
        assert!(!allowed_host("https://x@api.anthropic.com"));
    }

    #[test]
    fn decrypts_chromium_v10() {
        let key = [7u8; 32];
        let nonce = [1u8; 12];
        let sealed = Aes256Gcm::new_from_slice(&key).unwrap().encrypt(Nonce::from_slice(&nonce), b"{\"a\":1}".as_ref()).unwrap();
        let data = [b"v10".as_ref(), &nonce, &sealed].concat();
        assert_eq!(decrypt(&key, &data).unwrap(), b"{\"a\":1}");
        assert!(decrypt(&[8u8; 32], &data).is_none());
    }

    #[test]
    fn parses_usage_and_records_it() {
        let body = json!({ "five_hour": { "utilization": 23.0, "resets_at": "2026-11-26T15:00:00+00:00" }, "seven_day": { "utilization": 54.5, "resets_at": null }, "seven_day_opus": null });
        let windows = parse_usage(&body);
        assert_eq!(windows, vec![("five_hour".into(), 23.0, Some("2026-11-26T15:00:00+00:00".into())), ("seven_day".into(), 54.5, None)]);
        let dir = std::env::temp_dir().join(format!("claude-accounts-live-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        write_json(&dir.join(format!("{A}.json")), &json!({ "email": "a@b.ch", "orgId": "o" })).unwrap();
        record(&dir, A, &windows, 1000).unwrap();
        let v = read_json(&dir.join(format!("{A}.json"))).unwrap();
        assert_eq!((v["email"].as_str(), v["orgId"].as_str(), v["measuredAt"].as_i64()), (Some("a@b.ch"), Some("o"), Some(1000)));
        assert_eq!(v["limits"][0]["percentUsed"], 23.0);
        let _ = std::fs::remove_dir_all(dir);
    }
}
