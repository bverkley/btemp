//! Heuristic grouping of sensor readings into UI panels.

use crate::sensors::{stable_series_id, SensorReading};
use crate::storage_names::storage_drive_label;

/// High-level panel bucket for layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PanelKind {
    Cpu,
    Storage,
    Gpu,
    Motherboard,
    Other,
}

/// One logical series shown in the UI (may share a panel).
#[derive(Debug, Clone)]
pub struct SeriesSpec {
    pub id: String,
    pub chip: String,
    pub label: String,
    /// Shown in the UI (e.g. NVMe model from sysfs).
    pub display_name: String,
    pub max_c: Option<f64>,
    pub crit_c: Option<f64>,
    /// CPU: composite graph vs per-core strip.
    pub cpu_role: Option<CpuRole>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuRole {
    Composite,
    Core,
}

/// One NVMe (or storage chip) with composite vs auxiliary sensors for layout.
#[derive(Debug, Clone)]
pub struct StorageDriveSpec {
    /// Model from sysfs (or chip); shown once as a subsection title.
    pub display_name: String,
    pub chip: String,
    pub composite: Option<SeriesSpec>,
    /// Non-composite sensors (Sensor 2, etc.); right column.
    pub sensors: Vec<SeriesSpec>,
}

#[derive(Debug, Clone)]
pub struct PanelSpec {
    pub kind: PanelKind,
    pub title: &'static str,
    pub series: Vec<SeriesSpec>,
    /// Filled for `PanelKind::Storage` to drive per-drive composite layout.
    pub storage_drives: Vec<StorageDriveSpec>,
}

/// `label` is exactly `composite` (any ASCII case).
pub(crate) fn label_is_exact_composite(label: &str) -> bool {
    label.eq_ignore_ascii_case("composite")
}

/// `label` contains the substring `composite` (ASCII lowercase comparison).
pub(crate) fn label_contains_composite_substring(label: &str) -> bool {
    label.to_lowercase().contains("composite")
}

/// Left-column title under a drive header: unified "Composite" when the sensor name looks composite, else a short raw label.
pub(crate) fn composite_storage_row_title(label: &str) -> String {
    if label_is_exact_composite(label) || label_contains_composite_substring(label) {
        "Composite".to_string()
    } else {
        truncate_label_chars(label, 12)
    }
}

fn truncate_label_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    s.chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>()
        + "…"
}

fn partition_composite_for_chip(mut specs: Vec<SeriesSpec>) -> (Option<SeriesSpec>, Vec<SeriesSpec>) {
    specs.sort_by(|a, b| a.label.cmp(&b.label));
    if specs.is_empty() {
        return (None, vec![]);
    }
    // Prefer an exact "composite" feature name, else the first label that merely contains "composite"
    // (order differs from a single combined predicate when both kinds exist).
    let idx = specs
        .iter()
        .position(|s| label_is_exact_composite(&s.label))
        .or_else(|| {
            specs
                .iter()
                .position(|s| label_contains_composite_substring(&s.label))
        });
    if let Some(i) = idx {
        let c = specs.remove(i);
        return (Some(c), specs);
    }
    if specs.len() == 1 {
        let c = specs.remove(0);
        return (Some(c), vec![]);
    }
    let c = specs.remove(0);
    (Some(c), specs)
}

fn build_storage_drives(series: &[SeriesSpec]) -> Vec<StorageDriveSpec> {
    let mut by_chip: std::collections::BTreeMap<String, Vec<SeriesSpec>> =
        std::collections::BTreeMap::new();
    for s in series {
        by_chip
            .entry(s.chip.clone())
            .or_default()
            .push(s.clone());
    }
    let mut out = Vec::new();
    for (chip, specs) in by_chip {
        let display_name = specs
            .first()
            .map(|s| s.display_name.clone())
            .unwrap_or_else(|| storage_drive_label(&chip));
        let (composite, sensors) = partition_composite_for_chip(specs);
        out.push(StorageDriveSpec {
            display_name,
            chip,
            composite,
            sensors,
        });
    }
    out
}

/// Classify a single reading for panel bucketing (library hook; same rules as `group_readings`).
pub fn classify_panel_kind(r: &SensorReading) -> PanelKind {
    classify_reading(r)
}

fn classify_reading(r: &SensorReading) -> PanelKind {
    let a = r.adapter.to_lowercase();
    let chip = r.chip.to_lowercase();
    let label = r.label.to_lowercase();
    let hay = format!("{a} {chip} {label}");
    if hay.contains("k10temp")
        || hay.contains("coretemp")
        || hay.contains("zenpower")
        || hay.contains("cpu thermal")
        || hay.contains("cpu diode")
    {
        return PanelKind::Cpu;
    }
    if hay.contains("nvme") {
        return PanelKind::Storage;
    }
    if hay.contains("amdgpu")
        || hay.contains("radeon")
        || hay.contains("nouveau")
        || hay.contains("nvidia")
        || hay.contains("gpu")
    {
        return PanelKind::Gpu;
    }
    if hay.contains("acpitz")
        || hay.contains("nct")
        || hay.contains("it87")
        || hay.contains("asus")
        || hay.contains("super i/o")
        || hay.contains("sio")
        || hay.contains("pch")
    {
        return PanelKind::Motherboard;
    }
    PanelKind::Other
}

fn cpu_role_for_label(label: &str) -> CpuRole {
    let l = label.to_lowercase();
    if l.contains("tctl")
        || l.contains("tdie")
        || l.contains("package")
        || l.contains("socket")
        || l == "cpu"
    {
        return CpuRole::Composite;
    }
    if l.contains("core")
        || l.contains("ccd")
        || l.contains("tccd")
        || l.contains("ccd#")
    {
        return CpuRole::Core;
    }
    CpuRole::Composite
}

/// Build panel list: one panel per kind that has data, stable ordering.
pub fn group_readings(readings: &[SensorReading]) -> Vec<PanelSpec> {
    use PanelKind::*;
    let mut by_kind: std::collections::HashMap<PanelKind, Vec<SeriesSpec>> =
        std::collections::HashMap::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for r in readings {
        let id = stable_series_id(r);
        if !seen.insert(id.clone()) {
            continue;
        }
        let kind = classify_reading(r);
        let cpu_role = if kind == Cpu {
            Some(cpu_role_for_label(&r.label))
        } else {
            None
        };
        let display_name = if kind == PanelKind::Storage {
            storage_drive_label(&r.chip)
        } else {
            r.label.clone()
        };
        let spec = SeriesSpec {
            id,
            chip: r.chip.clone(),
            label: r.label.clone(),
            display_name,
            max_c: r.max_c,
            crit_c: r.crit_c,
            cpu_role,
        };
        by_kind.entry(kind).or_default().push(spec);
    }

    let order = [Cpu, Storage, Gpu, Motherboard, Other];
    let titles = [
        (Cpu, "CPU"),
        (Storage, "Storage"),
        (Gpu, "GPU"),
        (Motherboard, "Motherboard"),
        (Other, "Other"),
    ];
    let mut panels = Vec::new();
    for k in order {
        if let Some(mut series) = by_kind.remove(&k) {
            if k == Cpu {
                series.sort_by(|a, b| {
                    use CpuRole::*;
                    let ra = a.cpu_role.unwrap_or(Composite);
                    let rb = b.cpu_role.unwrap_or(Composite);
                    match (ra, rb) {
                        (Composite, Core) => std::cmp::Ordering::Less,
                        (Core, Composite) => std::cmp::Ordering::Greater,
                        _ => a.label.cmp(&b.label),
                    }
                });
            } else {
                series.sort_by(|a, b| a.chip.cmp(&b.chip).then_with(|| a.label.cmp(&b.label)));
            }
            let title = titles.iter().find(|(x, _)| *x == k).map(|(_, t)| *t).unwrap();
            let storage_drives = if k == Storage {
                build_storage_drives(&series)
            } else {
                vec![]
            };
            panels.push(PanelSpec {
                kind: k,
                title,
                series,
                storage_drives,
            });
        }
    }
    panels
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sensors::parse_json;

    const K10_JSON: &str = include_str!("../tests/fixtures/k10temp.json");
    const NVME_JSON: &str = include_str!("../tests/fixtures/nvme.json");

    #[test]
    fn classify_panel_kind_matches_grouping() {
        let r = parse_json(K10_JSON).unwrap();
        let tctl = r.iter().find(|x| x.label == "Tctl").unwrap();
        assert_eq!(classify_panel_kind(tctl), PanelKind::Cpu);
    }

    #[test]
    fn cpu_and_storage_panels() {
        let mut r = parse_json(K10_JSON).unwrap();
        r.extend(parse_json(NVME_JSON).unwrap());
        let panels = group_readings(&r);
        assert!(panels.iter().any(|p| p.kind == PanelKind::Cpu));
        let cpu = panels.iter().find(|p| p.kind == PanelKind::Cpu).unwrap();
        let composites: Vec<_> = cpu
            .series
            .iter()
            .filter(|s| s.cpu_role == Some(CpuRole::Composite))
            .collect();
        assert!(!composites.is_empty());
        let storage = panels.iter().find(|p| p.kind == PanelKind::Storage);
        assert!(storage.is_some());
    }

    #[test]
    fn storage_drives_split_composite_and_aux() {
        let r = parse_json(NVME_JSON).unwrap();
        let panels = group_readings(&r);
        let st = panels.iter().find(|p| p.kind == PanelKind::Storage).unwrap();
        let d0 = st
            .storage_drives
            .iter()
            .find(|d| d.chip == "nvme-pci-0100")
            .unwrap();
        assert!(d0.composite.as_ref().is_some_and(|c| c.label == "Composite"));
        assert!(d0.sensors.iter().any(|s| s.label == "Sensor 2"));
    }
}
