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

/// Desktop's newest usage sample of one organization.
#[derive(Clone, Debug, Default)]
pub struct Sample {
    pub at: i64,
    /// code -> percent
    pub figures: HashMap<String, f64>,
    /// code -> the first sample of the window still open at `at`. The window began
    /// before it, so it resets at the latest one window length after.
    pub opened: HashMap<String, i64>,
}

/// Desktop's newest usage sample per organization, with when each window was first seen.
pub fn history(user_data: &Path) -> HashMap<String, Sample> {
    let mut series: HashMap<String, Vec<(i64, HashMap<String, f64>)>> = HashMap::new();
    let Some(samples) = read_json(&user_data.join("plan-usage-history.json")).and_then(|v| v.get("samples").cloned()) else { return HashMap::new() };
    for s in samples.as_array().into_iter().flatten() {
        let (Some(t), Some(org), Some(u)) = (s.get("t").and_then(Value::as_f64), s.get("org").and_then(Value::as_str), s.get("u").and_then(Value::as_object)) else { continue };
        let figures = u.iter().filter_map(|(k, v)| Some((k.clone(), v.as_f64()?))).collect();
        series.entry(org.to_string()).or_default().push((t as i64, figures));
    }
    series.into_iter().filter_map(|(org, mut list)| {
        list.sort_by_key(|(t, _)| *t);
        Some((org, sample_of(&list)?))
    }).collect()
}

/// The newest sample of one organization's series, and when its windows opened.
///
/// A window ends one length after its first request. Between two samples it ended if the
/// figure dropped, if the samples lie a window length apart, or if the window seen open
/// is older than its length; the next sample above 0 then belongs to a new window.
fn sample_of(list: &[(i64, HashMap<String, f64>)]) -> Option<Sample> {
    let (at, figures) = list.last()?.clone();
    let mut opened = HashMap::new();
    for (_, code, length) in KINDS {
        let mut open: Option<i64> = None;
        let mut prev: Option<(i64, f64)> = None;
        for (t, u) in list {
            let Some(p) = u.get(code).copied() else { continue };
            open = if p <= 0.0 {
                None
            } else {
                match (open, prev) {
                    (Some(start), Some((pt, pp))) if p >= pp && t - pt < length && t - start < length => Some(start),
                    _ => Some(*t),
                }
            };
            prev = Some((*t, p));
        }
        if let Some(start) = open {
            opened.insert(code.to_string(), start);
        }
    }
    Some(Sample { at, figures, opened })
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Window {
    pub kind: String,
    /// Used share now, 0-100; `None` without any reading.
    pub percent: Option<f64>,
    pub resets_at: Option<i64>,
    /// Without an exact reset time: the latest it can be.
    pub resets_by: Option<i64>,
    /// The window has reset since the reading.
    pub is_reset: bool,
    /// When the reading was taken.
    pub at: Option<i64>,
}

/// One account's windows as of `now`.
pub fn windows(rec: Option<&Recorded>, sample: Option<&Sample>, now: i64) -> Vec<Window> {
    KINDS
        .iter()
        .map(|(kind, code, length)| {
            let from_mod = rec.and_then(|r| r.windows.get(*kind));
            let from_desktop = sample.and_then(|s| s.figures.get(*code).map(|p| (s.at, *p)));
            // Desktop's reading ends one window length after the window was first seen, at the latest.
            let desktop_by = |t: i64| sample.and_then(|s| s.opened.get(*code)).copied().unwrap_or(t) + length;
            let (reading, by) = match (from_mod, from_desktop) {
                (Some(m), Some((t, p))) if t > m.at => {
                    // The mod's reset time still holds if the window it names was open at the sample.
                    let resets_at = m.resets_at.filter(|r| *r > t);
                    (Some(Reading { percent: p, resets_at, at: t }), desktop_by(t))
                }
                (Some(m), _) => (Some(m.clone()), m.at + length),
                (None, Some((t, p))) => (Some(Reading { percent: p, resets_at: None, at: t }), desktop_by(t)),
                (None, None) => (None, 0),
            };
            match reading {
                None => Window { kind: kind.to_string(), percent: None, resets_at: None, resets_by: None, is_reset: false, at: None },
                Some(r) => {
                    let is_reset = match r.resets_at {
                        Some(reset) => reset <= now,
                        None => by <= now,
                    };
                    Window {
                        kind: kind.to_string(),
                        percent: Some(if is_reset { 0.0 } else { r.percent }),
                        resets_at: r.resets_at.filter(|_| !is_reset),
                        // At 0 % without a reset time no window is open: nothing to reset.
                        resets_by: (r.resets_at.is_none() && !is_reset && r.percent > 0.0).then_some(by),
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

    fn sample(t: i64, fh: f64) -> Sample {
        sample_of(&[point(t, fh)]).unwrap()
    }

    fn point(t: i64, fh: f64) -> (i64, HashMap<String, f64>) {
        (t, HashMap::from([("fh".to_string(), fh), ("sd".to_string(), 50.0)]))
    }

    #[test]
    fn reset_is_due_one_length_after_the_window_was_first_seen() {
        let now = 100 * H;
        // Old window up to 60 %, a drop: the new window was first seen at now-3h.
        let s = sample_of(&[point(now - 9 * H, 20.0), point(now - 6 * H, 60.0), point(now - 3 * H, 5.0), point(now - H, 30.0), point(now - 60_000, 42.0)]).unwrap();
        assert_eq!(s.opened["fh"], now - 3 * H);
        let w = windows(None, Some(&s), now);
        assert_eq!((w[0].percent, w[0].resets_by), (Some(42.0), Some(now + 2 * H)));
        // A sample at 0 % closes it; a gap of a window length too.
        let s = sample_of(&[point(now - 4 * H, 10.0), point(now - 2 * H, 0.0), point(now - H, 8.0)]).unwrap();
        assert_eq!(s.opened["fh"], now - H);
        let s = sample_of(&[point(now - 12 * H, 10.0), point(now - 6 * H, 15.0), point(now - H, 18.0)]).unwrap();
        assert_eq!(s.opened["fh"], now - H);
        // Past the latest reset the window reads 0.
        let s = sample_of(&[point(now - 6 * H, 10.0), point(now - 5 * H + 60_000, 30.0)]).unwrap();
        assert!(windows(None, Some(&s), now)[0].is_reset);
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