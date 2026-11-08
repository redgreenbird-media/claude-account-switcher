//! Names, e-mails and organizations of accounts, read from the account data
//! claude.ai caches in the web view's IndexedDB blobs.

use crate::claude::{is_uuid, read_shared};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Info {
    pub email: String,
    #[serde(default)]
    pub name: String,
    /// The account's first organization (the one its usage is counted under).
    #[serde(default)]
    pub org: String,
}

/// Looks for `id` in the blobs under `data_dir`.
pub fn find(data_dir: &Path, id: &str) -> Option<Info> {
    let blobs = data_dir.join("IndexedDB").join("https_claude.ai_0.indexeddb.blob");
    let mut stack = vec![blobs];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                stack.push(path);
            } else if meta.len() < 8 * 1024 * 1024 {
                if let Some(info) = read_shared(&path).ok().and_then(|b| parse(&b, id)) {
                    return Some(info);
                }
            }
        }
    }
    None
}

fn index_of(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if from >= hay.len() {
        return None;
    }
    hay[from..].windows(needle.len()).position(|w| w == needle).map(|i| i + from)
}

/// A V8-serialized one-byte string at `at`: `"`, varint length, bytes.
fn v8_string(b: &[u8], at: usize) -> Option<String> {
    if *b.get(at)? != b'"' {
        return None;
    }
    let (mut len, mut shift, mut i) = (0usize, 0, at + 1);
    loop {
        let c = *b.get(i)?;
        i += 1;
        len |= ((c & 0x7f) as usize) << shift;
        if c & 0x80 == 0 {
            break;
        }
        shift += 7;
        if shift > 28 {
            return None;
        }
    }
    if len == 0 || len > 200 || i + len > b.len() {
        return None;
    }
    Some(String::from_utf8_lossy(&b[i..i + len]).into_owned())
}

fn first_uuid(b: &[u8], from: usize, to: usize) -> Option<String> {
    let to = to.min(b.len());
    (from..to.saturating_sub(35)).find_map(|i| {
        let s = std::str::from_utf8(&b[i..i + 36]).ok()?;
        is_uuid(s).then(|| s.to_string())
    })
}

pub fn parse(b: &[u8], id: &str) -> Option<Info> {
    let mut from = 0;
    while let Some(at) = index_of(b, id.as_bytes(), from) {
        from = at + 1;
        let Some(mail) = index_of(b, b"email_address", at).filter(|m| m - at < 200) else { continue };
        let Some(email) = v8_string(b, mail + 13).filter(|e| e.contains('@')) else { continue };
        let name = index_of(b, b"full_name", mail)
            .filter(|n| n - mail < 300)
            .and_then(|n| v8_string(b, n + 9))
            .unwrap_or_default();
        let org = index_of(b, b"memberships", mail)
            .filter(|m| m - mail < 600)
            .and_then(|m| first_uuid(b, m, m + 400))
            .unwrap_or_default();
        return Some(Info { email, name, org });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(text: &str) -> Vec<u8> {
        let mut v = vec![b'"', text.len() as u8];
        v.extend_from_slice(text.as_bytes());
        v
    }

    #[test]
    fn reads_account_entry() {
        let id = "648f1ecd-7746-44a3-b93d-ca4acfb8f510";
        let org = "853dee05-435a-4cfd-9a9d-932405447d82";
        let mut b = b"junk\"uuid".to_vec();
        b.extend(s(id));
        b.extend_from_slice(b"\"email_address");
        b.extend(s("a@example.com"));
        b.extend_from_slice(b"\"full_name");
        b.extend(s("Ada Example"));
        b.extend_from_slice(b"\"membershipsa\x01o\"organiza\x00\"id\x10");
        b.extend_from_slice(org.as_bytes());
        let info = parse(&b, id).unwrap();
        assert_eq!(info, Info { email: "a@example.com".into(), name: "Ada Example".into(), org: org.into() });
    }

    #[test]
    fn ignores_id_without_email_nearby() {
        let id = "648f1ecd-7746-44a3-b93d-ca4acfb8f510";
        let b = format!("{id}{}", "x".repeat(400));
        assert!(parse(b.as_bytes(), id).is_none());
    }
}
