//! Resolve human-readable NVMe drive labels from sysfs (`/sys/class/nvme/*/model` + PCI address).

use std::fs;

/// Best-effort label for a storage hwmon chip (e.g. `nvme-pci-0100`).
pub fn storage_drive_label(chip: &str) -> String {
    nvme_model_for_chip(chip).unwrap_or_else(|| chip.to_string())
}

fn nvme_model_for_chip(chip: &str) -> Option<String> {
    let (bus, dev) = pci_bus_dev_from_nvme_chip(chip)?;
    let dir = fs::read_dir("/sys/class/nvme").ok()?;
    for ent in dir.flatten() {
        let name = ent.file_name().to_string_lossy().into_owned();
        let Some(rest) = name.strip_prefix("nvme") else {
            continue;
        };
        if rest.is_empty() || !rest.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let path = ent.path();
        let addr = fs::read_to_string(path.join("address")).ok()?;
        if pci_address_matches_bus_dev(&addr, bus, dev) {
            let model = fs::read_to_string(path.join("model")).ok()?;
            let m = model.trim().to_string();
            if !m.is_empty() {
                return Some(m);
            }
        }
    }
    None
}

/// Parse `nvme-pci-0100` style names: first two hex digits = PCI bus, next two = device (see `0000:01:00.0`).
fn pci_bus_dev_from_nvme_chip(chip: &str) -> Option<(u8, u8)> {
    let s = chip.to_lowercase();
    let rest = s.strip_prefix("nvme-pci-")?;
    let key = if rest.len() > 4 {
        &rest[rest.len() - 4..]
    } else {
        rest
    };
    if key.len() < 4 {
        return None;
    }
    let bus = u8::from_str_radix(&key[0..2], 16).ok()?;
    let dev = u8::from_str_radix(&key[2..4], 16).ok()?;
    Some((bus, dev))
}

fn pci_address_matches_bus_dev(address: &str, bus: u8, dev: u8) -> bool {
    let parts: Vec<&str> = address.trim().split(':').collect();
    if parts.len() < 3 {
        return false;
    }
    // Sysfs uses hex for BDF segments, e.g. `0000:65:00.0`.
    let Some(ad_bus) = u8::from_str_radix(parts[parts.len() - 2], 16).ok() else {
        return false;
    };
    let devfn = parts[parts.len() - 1];
    let Some(dev_str) = devfn.split('.').next() else {
        return false;
    };
    let Some(ad_dev) = u8::from_str_radix(dev_str, 16).ok() else {
        return false;
    };
    ad_bus == bus && ad_dev == dev
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pci_address_parsing() {
        assert!(pci_address_matches_bus_dev("0000:01:00.0", 1, 0));
        assert!(pci_address_matches_bus_dev("0000:65:00.0", 0x65, 0));
        assert!(!pci_address_matches_bus_dev("0000:01:00.0", 2, 0));
    }

    #[test]
    fn chip_suffix_parsing() {
        assert_eq!(pci_bus_dev_from_nvme_chip("nvme-pci-0100"), Some((1, 0)));
        assert_eq!(pci_bus_dev_from_nvme_chip("NVME-PCI-6500"), Some((0x65, 0)));
        assert_eq!(pci_bus_dev_from_nvme_chip("nvme-pci-deadbeef0100"), Some((1, 0)));
    }
}
