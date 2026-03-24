//! Parse `sensors -j` JSON and plain `sensors` text into normalized temperature readings.
//! We only keep values from keys like `tempN_input` (JSON) or `°C` lines (text) to avoid voltage fans.

use anyhow::{Context, Result};
use serde_json::Value;
use std::process::Command;

/// One temperature sample from lm-sensors after normalization.
#[derive(Debug, Clone, PartialEq)]
pub struct SensorReading {
    pub chip: String,
    pub adapter: String,
    pub label: String,
    pub value_c: f64,
    pub max_c: Option<f64>,
    pub crit_c: Option<f64>,
}

/// Stable id for history buffers (chip + feature label).
pub fn stable_series_id(reading: &SensorReading) -> String {
    format!("{}::{}", reading.chip, reading.label)
}

/// Run `sensors -j` when possible, otherwise plain `sensors` text.
pub fn fetch_readings() -> Result<Vec<SensorReading>> {
    let json_out = Command::new("sensors").args(["-j"]).output();
    if let Ok(out) = &json_out {
        if out.status.success() && !out.stdout.is_empty() {
            let s = String::from_utf8_lossy(&out.stdout);
            if let Ok(v) = parse_json(&s) {
                if !v.is_empty() {
                    return Ok(v);
                }
            }
        }
    }
    let out = Command::new("sensors")
        .output()
        .context("failed to run `sensors`; install lm-sensors and ensure it is on PATH")?;
    anyhow::ensure!(
        out.status.success(),
        "sensors exited with status {}",
        out.status
    );
    parse_text(&String::from_utf8_lossy(&out.stdout))
}

/// Parse JSON produced by `sensors -j`.
pub fn parse_json(raw: &str) -> Result<Vec<SensorReading>> {
    let root: Value =
        serde_json::from_str(raw).context("invalid sensors JSON")?;
    let mut out = Vec::new();
    let obj = root
        .as_object()
        .context("sensors JSON root must be an object")?;
    for (chip, chip_val) in obj {
        let Some(chip_map) = chip_val.as_object() else {
            continue;
        };
        let adapter = chip_map
            .get("Adapter")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        for (feature, feat_val) in chip_map {
            if feature == "Adapter" {
                continue;
            }
            let Some(feat_map) = feat_val.as_object() else {
                continue;
            };
            for (k, v) in feat_map {
                let Some(num) = v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)) else {
                    continue;
                };
                let Some(suffix) = k.strip_prefix("temp") else {
                    continue;
                };
                let Some((idx, rest)) = suffix.split_once('_') else {
                    continue;
                };
                if !idx.chars().all(|c| c.is_ascii_digit()) || rest != "input" {
                    continue;
                }
                let prefix = format!("temp{idx}");
                let max_c = feat_map
                    .get(&format!("{prefix}_max"))
                    .and_then(|x| x.as_f64().or_else(|| x.as_i64().map(|i| i as f64)));
                let crit_c = feat_map
                    .get(&format!("{prefix}_crit"))
                    .and_then(|x| x.as_f64().or_else(|| x.as_i64().map(|i| i as f64)));
                out.push(SensorReading {
                    chip: chip.clone(),
                    adapter: adapter.clone(),
                    label: feature.clone(),
                    value_c: num,
                    max_c,
                    crit_c,
                });
                // One `temp*_input` per feature is enough for this TUI; lm-sensors may list other temp keys on the same feature.
                break;
            }
        }
    }
    Ok(out)
}

/// Parse classic `sensors` text output (no `-j`).
pub fn parse_text(raw: &str) -> Result<Vec<SensorReading>> {
    let mut readings = Vec::new();
    let mut chip = String::new();
    let mut adapter = String::new();

    for line in raw.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            chip.clear();
            adapter.clear();
            continue;
        }
        if let Some(rest) = line.strip_prefix("Adapter:") {
            adapter = rest.trim().to_string();
            continue;
        }
        if !line.contains(':') || !line.contains('°') {
            if chip.is_empty() && !line.starts_with(char::is_whitespace) {
                chip = line.trim().to_string();
            }
            continue;
        }
        if let Some(r) = parse_text_sensor_line(line) {
            if chip.is_empty() {
                continue;
            }
            readings.push(SensorReading {
                chip: chip.clone(),
                adapter: adapter.clone(),
                label: r.label,
                value_c: r.value_c,
                max_c: r.max_c,
                crit_c: r.crit_c,
            });
        }
    }
    Ok(readings)
}

struct TextParsed {
    label: String,
    value_c: f64,
    max_c: Option<f64>,
    crit_c: Option<f64>,
}

fn parse_text_sensor_line(line: &str) -> Option<TextParsed> {
    let (left, right) = line.split_once(':')?;
    let label = left.trim().to_string();
    let rest = right.trim();
    let deg = rest.find('°')?;
    let num_part = rest[..deg].trim().trim_start_matches('+');
    let value_c = num_part.parse::<f64>().ok()?;
    let mut max_c = None;
    let mut crit_c = None;
    if let Some(open) = rest.find('(') {
        if let Some(close) = rest.rfind(')') {
            let inside = &rest[open + 1..close];
            for part in inside.split(',') {
                let p = part.trim();
                if let Some(v) = parse_temp_assignment(p, "high") {
                    max_c = Some(v);
                } else if let Some(v) = parse_temp_assignment(p, "crit") {
                    crit_c = Some(v);
                }
            }
        }
    }
    Some(TextParsed {
        label,
        value_c,
        max_c,
        crit_c,
    })
}

fn parse_temp_assignment(part: &str, key: &str) -> Option<f64> {
    let needle = format!("{key} =");
    let idx = part.find(&needle)?;
    let tail = part[idx + needle.len()..].trim();
    let tail = tail.trim_start_matches('+');
    let end = tail.find('°').unwrap_or(tail.len());
    tail[..end].trim().parse::<f64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const K10_JSON: &str = include_str!("../tests/fixtures/k10temp.json");
    const NVME_JSON: &str = include_str!("../tests/fixtures/nvme.json");
    const MIXED_TXT: &str = include_str!("../tests/fixtures/k10temp.txt");

    #[test]
    fn json_k10temp_parses_tctl_and_ccds() {
        let r = parse_json(K10_JSON).unwrap();
        assert!(r.iter().any(|x| x.label == "Tctl" && (x.value_c - 45.5).abs() < 0.01));
        assert_eq!(
            r.iter().find(|x| x.label == "Tctl").unwrap().crit_c,
            Some(100.0)
        );
        assert!(r.iter().any(|x| x.label == "Tccd1"));
    }

    #[test]
    fn json_nvme_two_chips() {
        let r = parse_json(NVME_JSON).unwrap();
        assert_eq!(r.len(), 3);
        let c0 = r.iter().find(|x| x.chip == "nvme-pci-0100").unwrap();
        assert_eq!(c0.label, "Composite");
        assert!((c0.value_c - 38.9).abs() < 0.01);
    }

    #[test]
    fn text_mixed_matches_jsonish_counts() {
        let r = parse_text(MIXED_TXT).unwrap();
        assert!(r.len() >= 4);
        assert!(r.iter().any(|x| x.label == "Tctl"));
        assert!(r.iter().any(|x| x.chip.contains("nvme")));
    }
}
