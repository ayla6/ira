use std::fs;
use std::path::Path;
use std::sync::OnceLock;

#[derive(Debug, Clone)]
pub struct GpuInfo {
    pub card: String,
    pub driver: String,
    pub pci_id: String,
    pub name: String,
    pub is_nvidia: bool,
}

#[derive(Debug, Clone)]
struct VulkanDevice {
    pci_id: String,
    device_uuid: String,
}

static VULKAN_DEVICES: OnceLock<Vec<VulkanDevice>> = OnceLock::new();

fn vulkan_devices() -> &'static Vec<VulkanDevice> {
    VULKAN_DEVICES.get_or_init(vulkan_devices_uncached)
}

/// Enumerates Vulkan devices via `vulkaninfo --summary` and maps each to a
/// `vendor:device` PCI ID plus its device UUID (dashes stripped, lowercase).
/// Empty on failure so callers can fall back gracefully.
fn vulkan_devices_uncached() -> Vec<VulkanDevice> {
    let output = match std::process::Command::new("vulkaninfo")
        .arg("--summary")
        .output()
    {
        Ok(o) if o.status.success() => o.stdout,
        _ => return Vec::new(),
    };
    parse_vulkaninfo_summary(&String::from_utf8_lossy(&output))
}

fn parse_vulkaninfo_summary(output: &str) -> Vec<VulkanDevice> {
    let mut devices = Vec::new();
    let mut current: Option<(String, String)> = None;
    for line in output.lines() {
        let line = line.trim();
        if line.starts_with("GPU") {
            if let Some((pci_id, uuid)) = current.take() {
                if !pci_id.is_empty() && !uuid.is_empty() {
                    devices.push(VulkanDevice {
                        pci_id,
                        device_uuid: uuid,
                    });
                }
            }
            current = Some((String::new(), String::new()));
            continue;
        }
        let Some((key, value)) = line.split_once("= ") else {
            continue;
        };
        let Some((pci_id, uuid)) = current.as_mut() else {
            continue;
        };
        match key.trim() {
            "vendorID" | "deviceID" => {
                let num =
                    u64::from_str_radix(value.trim().trim_start_matches("0x"), 16).unwrap_or(0);
                if key.trim() == "vendorID" {
                    pci_id.push_str(&format!("{:04x}:", num));
                } else {
                    pci_id.push_str(&format!("{:04x}", num));
                }
            }
            "deviceUUID" => uuid.push_str(&value.trim().replace('-', "").to_lowercase()),
            _ => {}
        }
    }
    if let Some((pci_id, uuid)) = current.take() {
        if !pci_id.is_empty() && !uuid.is_empty() {
            devices.push(VulkanDevice {
                pci_id,
                device_uuid: uuid,
            });
        }
    }
    devices
}

impl GpuInfo {
    /// A display-friendly GPU name: strips the vendor corporation prefix
    /// ("NVIDIA Corporation", "Intel Corporation", ...) and parenthetical
    /// suffixes like "(rev a2)" so both the app-wide and per-game GPU
    /// dropdowns show e.g. "GM108M [GeForce MX130]" instead.
    pub fn short_name(&self) -> String {
        const VENDOR_PREFIXES: [&str; 5] = [
            "NVIDIA Corporation",
            "Intel Corporation",
            "Advanced Micro Devices, Inc.",
            "AMD/ATI",
            "ATI Technologies Inc",
        ];
        let mut n = self.name.trim();
        for prefix in VENDOR_PREFIXES {
            if let Some(rest) = n.strip_prefix(prefix) {
                n = rest.trim_start();
                break;
            }
        }
        if let Some(idx) = n.find(" (") {
            n[..idx].trim().to_string()
        } else {
            n.to_string()
        }
    }
}

static GPU_CACHE: OnceLock<Vec<GpuInfo>> = OnceLock::new();

/// Detect all GPUs on the system by reading /sys/class/drm/.
/// Results are cached after the first call (GPUs don't change at runtime).
/// Returns a list of GpuInfo, one per cardN entry.
pub fn detect_gpus() -> Vec<GpuInfo> {
    GPU_CACHE.get_or_init(detect_gpus_uncached).clone()
}

fn detect_gpus_uncached() -> Vec<GpuInfo> {
    let drm_dir = Path::new("/sys/class/drm");
    let entries = match fs::read_dir(drm_dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut gpus: Vec<_> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name();
            let name_str = name.to_str()?;
            // Only match cardN (e.g. card0, card1), not card1-HDMI-A-1 or renderD128
            if !name_str.starts_with("card") || name_str.contains('-') || name_str.contains(':') {
                return None;
            }
            let suffix = &name_str[4..];
            if !suffix.chars().all(|c| c.is_ascii_digit()) || suffix.is_empty() {
                return None;
            }
            read_gpu_info(name_str)
        })
        .collect();
    gpus.sort_by(|a, b| a.card.cmp(&b.card));
    gpus
}

fn read_gpu_info(card: &str) -> Option<GpuInfo> {
    let uevent_path = format!("/sys/class/drm/{}/device/uevent", card);
    let uevent = fs::read_to_string(&uevent_path).ok()?;
    let mut driver = String::new();
    let mut pci_id = String::new();
    let mut pci_slot = String::new();
    for line in uevent.lines() {
        if let Some((k, v)) = line.split_once('=') {
            match k {
                "DRIVER" => driver = v.to_string(),
                "PCI_ID" => pci_id = v.to_string().to_lowercase(),
                "PCI_SLOT_NAME" => pci_slot = v.to_string(),
                _ => {}
            }
        }
    }
    let is_nvidia = driver == "nvidia";
    let name = get_gpu_name(&pci_slot).unwrap_or_else(|| format!("Unknown ({})", driver));
    Some(GpuInfo {
        card: card.to_string(),
        driver,
        pci_id,
        name,
        is_nvidia,
    })
}

fn get_gpu_name(pci_slot: &str) -> Option<String> {
    let output = std::process::Command::new("lspci")
        .arg("-D")
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let parts: Vec<&str> = line.splitn(2, ' ').collect();
        if parts.len() == 2 {
            let bus_id = parts[0];
            let desc = parts[1];
            if bus_id == pci_slot || format!("0000:{}", bus_id) == pci_slot {
                if let Some(name) = desc.split(": ").nth(1) {
                    return Some(name.to_string());
                }
                return Some(desc.to_string());
            }
        }
    }
    None
}

/// Build environment variables for GPU selection.
/// Returns a list of (key, value) pairs to set when launching with a specific GPU.
pub fn build_gpu_env(gpu_card: &str) -> Vec<(String, String)> {
    if gpu_card.is_empty() {
        return Vec::new();
    }
    let gpus = detect_gpus();
    let gpu = match gpus.iter().find(|g| g.card == gpu_card) {
        Some(g) => g,
        None => return Vec::new(),
    };
    let mut env = Vec::new();
    if gpu.is_nvidia {
        env.push(("DRI_PRIME".to_string(), "1".to_string()));
        env.push(("__NV_PRIME_RENDER_OFFLOAD".to_string(), "1".to_string()));
        env.push((
            "__GLX_VENDOR_LIBRARY_NAME".to_string(),
            "nvidia".to_string(),
        ));
        env.push((
            "__VK_LAYER_NV_optimus".to_string(),
            "NVIDIA_only".to_string(),
        ));
    } else {
        env.push(("DRI_PRIME".to_string(), gpu.pci_id.clone()));
    }
    let icd_files = find_icd_files(&gpu.driver);
    if !icd_files.is_empty() {
        env.push(("VK_ICD_FILENAMES".to_string(), icd_files.clone()));
        env.push(("VK_DRIVER_FILES".to_string(), icd_files));
    }
    // Pin the selected GPU in DXVK by device UUID (matches Lutris). Required on
    // multi-GPU (Optimus) systems where pressure-vessel injects both drivers —
    // otherwise DXVK may pick the iGPU, which breaks GPU-sensitive games.
    if let Some(uuid) = vulkan_devices()
        .iter()
        .find(|d| d.pci_id == gpu.pci_id)
        .map(|d| d.device_uuid.clone())
    {
        env.push(("DXVK_FILTER_DEVICE_UUID".to_string(), uuid));
    }
    env
}

fn find_icd_files(driver: &str) -> String {
    let loader = match driver {
        "amdgpu" => "radeon",
        "vc4-drm" => "broadcom",
        "v3d" => "broadcom",
        "virtio-pci" => "lvp",
        "i915" => "intel",
        "xe" => "intel",
        other => other,
    };
    let search_dirs = [
        "/usr/local/etc/vulkan/icd.d",
        "/usr/local/share/vulkan/icd.d",
        "/etc/vulkan/icd.d",
        "/usr/share/vulkan/icd.d",
        "/usr/lib/x86_64-linux-gnu/vulkan/icd.d",
        "/usr/lib64/vulkan/icd.d",
        "/opt/amdgpu-pro/etc/vulkan/icd.d",
    ];
    let mut files = Vec::new();
    for dir in &search_dirs {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "json") {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        if name.contains(loader) {
                            files.push(path.to_string_lossy().to_string());
                        }
                    }
                }
            }
        }
    }
    files.join(":")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_short_name_strips_parens() {
        let gpu = GpuInfo {
            card: "card0".to_string(),
            driver: "amdgpu".to_string(),
            pci_id: "1002:73bf".to_string(),
            name: "AMD Radeon Pro W6800 (RADV NAVI21)".to_string(),
            is_nvidia: false,
        };
        assert_eq!(gpu.short_name(), "AMD Radeon Pro W6800");
    }

    #[test]
    fn test_short_name_no_parens() {
        let gpu = GpuInfo {
            card: "card0".to_string(),
            driver: "nvidia".to_string(),
            pci_id: "10de:13c2".to_string(),
            name: "GeForce GTX 970".to_string(),
            is_nvidia: true,
        };
        assert_eq!(gpu.short_name(), "GeForce GTX 970");
    }

    #[test]
    fn test_short_name_strips_corporation_prefix() {
        let gpu = GpuInfo {
            card: "card0".to_string(),
            driver: "nvidia".to_string(),
            pci_id: "10de:174d".to_string(),
            name: "NVIDIA Corporation GM108M [GeForce MX130] (rev a2)".to_string(),
            is_nvidia: true,
        };
        assert_eq!(gpu.short_name(), "GM108M [GeForce MX130]");

        let gpu = GpuInfo {
            card: "card1".to_string(),
            driver: "i915".to_string(),
            pci_id: "8086:3ea0".to_string(),
            name: "Intel Corporation WhiskeyLake-U GT2 [UHD Graphics 620] (rev 02)".to_string(),
            is_nvidia: false,
        };
        assert_eq!(gpu.short_name(), "WhiskeyLake-U GT2 [UHD Graphics 620]");
    }

    #[test]
    fn test_short_name_amd_prefix() {
        let gpu = GpuInfo {
            card: "card0".to_string(),
            driver: "amdgpu".to_string(),
            pci_id: "1002:73bf".to_string(),
            name: "Advanced Micro Devices, Inc. [AMD/ATI] NAVI 21 [Radeon RX 6800 XT]".to_string(),
            is_nvidia: false,
        };
        assert_eq!(gpu.short_name(), "[AMD/ATI] NAVI 21 [Radeon RX 6800 XT]");
    }

    #[test]
    fn test_build_gpu_env_empty() {
        let env = build_gpu_env("");
        assert!(env.is_empty());
    }

    #[test]
    fn test_build_gpu_env_nonexistent() {
        let env = build_gpu_env("card99");
        assert!(env.is_empty());
    }

    #[test]
    fn test_parse_vulkaninfo_summary_multiple_gpus() {
        let output = r#"Devices:
========
GPU0:
	apiVersion         = 0x420008
	vendorID           = 0x8086
	deviceID           = 0x3ea0
	deviceType         = INTEGRATED_GPU
	deviceName         = Intel(R) UHD Graphics 620 (WHL GT2)
	driverName         = Intel open-source Mesa driver
	driverInfo         = 24.0.9
	deviceUUID         = 8680a03e-0200-0000-0002-000000000000
GPU1:
	vendorID           = 0x10de
	deviceID           = 0x174d
	deviceType         = DISCRETE_GPU
	deviceName         = NVIDIA GeForce MX130
	deviceUUID         = 35433f11-93fc-0291-b19b-f9e05c68b2a0
"#;
        let devices = parse_vulkaninfo_summary(output);
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].pci_id, "8086:3ea0");
        assert_eq!(devices[0].device_uuid, "8680a03e020000000002000000000000");
        assert_eq!(devices[1].pci_id, "10de:174d");
        assert_eq!(devices[1].device_uuid, "35433f1193fc0291b19bf9e05c68b2a0");
    }

    #[test]
    fn test_parse_vulkaninfo_summary_no_gpus() {
        assert!(parse_vulkaninfo_summary("Failed to detect any valid GPUs").is_empty());
        assert!(parse_vulkaninfo_summary("").is_empty());
    }
}
