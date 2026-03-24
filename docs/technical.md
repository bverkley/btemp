# btemp technical notes

## Stack

- Rust (2021 edition), binary crate `btemp`
- `ratatui` + `crossterm` for terminal UI; braille graph logic lives in `src/ui/chart.rs`, event loop and layout in `src/ui/mod.rs`
- `serde` / `serde_json` for `sensors -j` output
- `anyhow` for error context in the main loop

`group::classify_panel_kind` exposes the same panel bucket as `group_readings` for a single reading (tests keep it from going stale). Graphs use ratatui `Canvas` with `Marker::Braille`: °C on Y, sample index on X. Y-bounds follow the **data min/max** (with padding and a minimum span) so flat traces still use the full chart height. **Horizontal scale:** X is always mapped to a fixed window of **`HISTORY_CAP` samples** (same as the ring buffer); the visible series is **right-aligned** in that window so short histories leave empty space on the left and, once full, the graph **scrolls** like btop instead of stretching fewer points across the full width. Panel rows use **`Constraint::Fill(1)`** so each graph row shares the panel’s vertical space (no fixed 8-row band). Interpolated vertical slices sample X across that window. **Fill colors** follow **btop-style vertical heat**: each drawn segment is tinted by its **normalized Y position in the chart** (bottom = dark green, mid = yellow/orange, top = red), so a tall column shows the full ramp. Numeric labels still use °C; `max_c` / `crit` are available on readings but are not currently used to skew the braille gradient.

CPU panel block title: first `model name` / `Model` field from `/proc/cpuinfo` (trimmed). Storage drive body: **`Percentage(67)` / `Percentage(33)`** (composite vs sensors). Labels use Unicode **°C**.

## Data contract

1. Prefer `sensors -j` (JSON). If stderr indicates unknown flag or parse fails, run `sensors` (plain text).
2. Normalized reading: chip adapter name, chip key, sensor label (or synthetic), value Celsius, optional max/crit from sibling keys (`*_max`, `*_crit`).

## Storage labels and layout

- **Names:** For chips named like `nvme-pci-0100`, map the last four hex digits to PCI bus/device and match `/sys/class/nvme/nvme*/address` (hex BDF), then read `model`. Fallback: raw chip string.
- **Layout (width ≥ 44):** Group by `chip`. Per drive: header = model once; body split **67% / 33%**: left = **Composite** (or first sensor), right = other sensors on that chip, equal row heights on the right.

## Grouping rules (v1)

- **CPU:** Adapter contains `k10temp`, `coretemp`, `zenpower`, or label matches package/Tctl/Tdie patterns.
- **Storage:** Adapter or label suggests `nvme` (case-insensitive).
- **GPU:** Adapter or label suggests `amdgpu`, `radeon`, `nouveau`, `nvidia`.
- **Motherboard:** `acpitz`, `nct`, `it87`, `asus`, `super io` style names.
- **Other:** Everything else.

CPU composite priority: Tctl, Tdie, Package, first high-level temp; per-core: labels matching `Core N`, `CCD`, `Tccd`, `Core`.

## Testing

- Fixture files under `tests/fixtures/` for JSON and text; unit tests do not shell out to real hardware in default CI runs.
