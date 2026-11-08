//! Each account's 5-hour and weekly limits, merged from two sources:
//!
//! - `plan-usage-history.json` in Claude Desktop's data folder: Desktop's own
//!   samples of the usage page (same figures as Settings › Usage), per
//!   organization, every few minutes while it runs. No reset times.
//! - `limits\<uuid>.json`, written by the account-switcher mod after each reply
//!   of a Code session: the rate-limit windows with their reset times.
//!
//! The newer reading wins per window; a reset time carries over while the
//! window it belongs to is still open.

use crate::claude::read_json;
use serde::Serialize;
use serde_json::Value;
use std::{collections::HashMap, fs, path::Path};

pub const KINDS: [(&str, &str, i64); 2] = [("five_hour", "fh", 5 * 3_600_000), ("seven_day", "sd", 7 * 86_400_000)];

pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// `2026-11-08 10:22:15` in local time, for the log.
pub fn stamp(ms: i64) -> String {
    let utc = time::OffsetDateTime::from_unix_timestamp_nanos(ms as i128 * 1_000_000).unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
    let t = time::UtcOffset::current_local_offset().map(|o| utc.to_offset(o)).unwrap_or(utc);
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", t.year(), t.month() as u8, t.day(), t.hour(), t.minute(), t.second())
}

fn parse_time(s: &str) -> Option<i64> {
    let t = time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339).ok()?;
    Some((t.unix_timestamp_nanos() / 1_000_000) as i64)
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reading {
    pub percent: f64,
    pub resets_at: Option<i64>,
    pub at: i64,
}

/// What the mod recorded for one account.
#[derive(Clone, Debug, Default)]
pub struct Recorded {
    pub email: String,
    pub org: String,
    pub windows: HashMap<String, Reading>,
}

fn recorded_from(v: &Value) -> Option<Recorded> {
    let at = v.get("measuredAt")?.as_f64()? as i64;
    let mut windows = HashMap::new();
    for l in v.get("limits")?.as_array()? {
        let (Some(kind), Some(percent)) = (l.get("kind").and_then(Value::as_str), l.get("percentUsed").and_then(Value::as_f64)) else { continue };
        let resets_at = l.get("resetsAt").and_then(Value::as_str).and_then(parse_time);
        windows.insert(kind.to_string(), Reading { percent, resets_at, at });
    }
    Some(Recorded {
        email: v.get("email").and_then(Value::as_str).unwrap_or_default().to_string(),
        org: v.get("orgId").and_then(Value::as_str).unwrap_or_default().to_string(),
        windows,
    })
}

/// The mod's records: `limits\<uuid>.json`, and accounts.json from mod 0.1.
pub fn recorded(root: &Path) -> HashMap<String, Recorded> {
    let mut out: HashMap<String, Recorded> = HashMap::new();
    let newest = |r: &Recorded| r.windows.values().map(|w| w.at).max().unwrap_or(0);
    let mut put = |id: String, r: Recorded| {
        if out.get(&id).map_or(true, |old| newest(&r) >= newest(old)) {
            out.insert(id, r);
        }
    };
    if let Some(Value::Object(accounts)) = read_json(&root.join("accounts.json")).and_then(|v| v.get("accounts").cloned()) {
        for (id, v) in accounts {
            if let Some(r) = recorded_from(&v) {
                put(id, r);
            }
        }
    }
    for e in fs::read_dir(root.join("limits")).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let Some(id) = name.strip_suffix(".json") else { continue };
        if let Some(r) = read_json(&e.path()).as_ref().and_then(recorded_from) {
            put(id.to_string(), r);
        }
    }
    out
}

/// Desktop's newest usage sample per organization: (when, code -> percent).
pub fn history(user_data: &Path) -> HashMap<String, (i64, HashMap<String, f64>)> {
    let mut out: HashMap<String, (i64, HashMap<String, f64>)> = HashMap::new();
    let Some(samples) = read_json(&user_data.join("plan-usage-history.json")).and_then(|v| v.get("samples").cloned()) else { return out };
    for s in samples.as_array().into_iter().flatten() {
        let (Some(t), Some(org), Some(u)) = (s.get("t").and_then(Value::as_f64), s.get("org").and_then(Value::as_str), s.get("u").and_then(Value::as_object)) else { continue };
        let t = t as i64;
        if out.get(org).map_or(true, |(old, _)| t >= *old) {
            let figures = u.iter().filter_map(|(k, v)| Some((k.clone(), v.as_f64()?))).collect();
            out.insert(org.to_string(), (t, figures));
        }
    }
    out
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Window {
    pub kind: String,
    /// Used share now, 0-100; `None` without any reading.
    pub percent: Option<f64>,
    pub resets_at: Option<i64>,
    /// The window has reset since the reading.
    pub is_reset: bool,
    /// When the reading was taken.
    pub at: Option<i64>,
}

/// One account's windows as of `now`.
pub fn windows(rec: Option<&Recorded>, sample: Option<&(i64, HashMap<String, f64>)>, now: i64) -> Vec<Window> {
    KINDS
        .iter()
        .map(|(kind, code, length)| {
            let from_mod = rec.and_then(|r| r.windows.get(*kind));
            let from_desktop = sample.and_then(|(t, u)| u.get(*code).map(|p| (*t, *p)));
            let reading = match (from_mod, from_desktop) {
                (Some(m), Some((t, p))) if t > m.at => Some(Reading {
                    percent: p,
                    // The mod's reset time still holds if the window it names was open at the sample.
                    resets_at: m.resets_at.filter(|r| *r > t),
                    at: t,
                }),
                (Some(m), _) => Some(m.clone()),
                (None, Some((t, p))) => Some(Reading { percent: p, resets_at: None, at: t }),
                (None, None) => None,
            };
            match reading {
                None => Window { kind: kind.to_string(), percent: None, resets_at: None, is_reset: false, at: None },
                Some(r) => {
                    let is_reset = match r.resets_at {
                        Some(reset) => reset <= now,
                        // Without a reset time: a window longer ago than its own length is over for sure.
                        None => now - r.at >= *length,
                    };
                    Window {
                        kind: kind.to_string(),
                        percent: Some(if is_reset { 0.0 } else { r.percent }),
                        resets_at: r.resets_at.filter(|_| !is_reset),
                        is_reset,
                        at: Some(r.at),
                    }
                }
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: i64 = 3_600_000;

    fn rec(pct: f64, reset: Option<i64>, at: i64) -> Recorded {
        let mut windows = HashMap::new();
        windows.insert("five_hour".into(), Reading { percent: pct, resets_at: reset, at });
        Recorded { windows, ..Default::default() }
    }

    fn sample(t: i64, fh: f64) -> (i64, HashMap<String, f64>) {
        (t, HashMap::from([("fh".to_string(), fh), ("sd".to_string(), 50.0)]))
    }

    #[test]
    fn newer_desktop_sample_wins_and_keeps_open_reset() {
        let now = 100 * H;
        let w = windows(Some(&rec(20.0, Some(now + 2 * H), now - H)), Some(&sample(now - 10, 35.0)), now);
        assert_eq!(w[0].percent, Some(35.0));
        assert_eq!(w[0].resets_at, Some(now + 2 * H));
        assert_eq!(w[1].percent, Some(50.0));
        assert_eq!(w[1].resets_at, None);
    }

    #[test]
    fn newer_mod_reading_wins() {
        let now = 100 * H;
        let w = windows(Some(&rec(40.0, Some(now + H), now - 60)), Some(&sample(now - H, 35.0)), now);
        assert_eq!(w[0].percent, Some(40.0));
    }

    #[test]
    fn passed_reset_reads_zero() {
        let now = 100 * H;
        let w = windows(Some(&rec(90.0, Some(now - 1), now - 3 * H)), None, now);
        assert_eq!((w[0].percent, w[0].is_reset), (Some(0.0), true));
    }

    #[test]
    fn old_sample_without_reset_is_over() {
        let now = 100 * H;
        let w = windows(None, Some(&sample(now - 6 * H, 100.0)), now);
        assert_eq!((w[0].percent, w[0].is_reset), (Some(0.0), true));
        assert_eq!((w[1].percent, w[1].is_reset), (Some(50.0), false));
    }

    #[test]
    fn reset_of_an_older_window_is_dropped() {
        let now = 100 * H;
        // Mod saw a window ending at now-2h; Desktop sampled after that, in the next window.
        let w = windows(Some(&rec(80.0, Some(now - 2 * H), now - 4 * H)), Some(&sample(now - H, 10.0)), now);
        assert_eq!((w[0].percent, w[0].resets_at, w[0].is_reset), (Some(10.0), None, false));
    }

    #[test]
    fn parses_mod_record() {
        let v: Value = serde_json::json!({
            "email": "a@b.ch", "orgId": "o", "measuredAt": 1000.0,
            "limits": [{ "kind": "five_hour", "percentUsed": 38, "resetsAt": "2026-11-08T13:10:00.000Z" }]
        });
        let r = recorded_from(&v).unwrap();
        assert_eq!(r.windows["five_hour"].resets_at, Some(1_794_143_400_000));
        assert_eq!(r.org, "o");
    }
}