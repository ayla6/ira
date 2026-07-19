use std::fs;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct GpuInfo {
    pub card: String,
    pub driver: String,
    pub pci_id: String,
    pub name: String,
    pub is_nvidia: bool,
}

impl GpuInfo {
    pub fn short_name(&self) -> String {
        let n = self.name.trim();
        if let Some(idx) = n.find(" (") {
            n[..idx].trim().to_string()
        } else {
            n.to_string()
        }
    }
}

/// Detect all GPUs on the system by reading /sys/class/drm/.
/// Returns a list of GpuInfo, one per cardN entry.
pub fn detect_gpus() -> Vec<GpuInfo> {
    let drm_dir = Path::new("/sys/class/drm");
    let entries = match fs::read_dir(drm_dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut gpus = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = match name.to_str() {
            Some(s) => s,
            None => continue,
        };
        if !name_str.starts_with("card") || name_str.contains(':') {
            continue;
        }
        if let Some(gpu) = read_gpu_info(name_str) {
            gpus.push(gpu);
        }
    }
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
        env.push(("__GLX_VENDOR_LIBRARY_NAME".to_string(), "nvidia".to_string()));
        env.push(("__VK_LAYER_NV_optimus".to_string(), "NVIDIA_only".to_string()));
    } else {
        env.push(("DRI_PRIME".to_string(), gpu.pci_id.clone()));
    }
    let icd_files = find_icd_files(&gpu.driver);
    if !icd_files.is_empty() {
        env.push(("VK_ICD_FILENAMES".to_string(), icd_files.clone()));
        env.push(("VK_DRIVER_FILES".to_string(), icd_files));
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
    fn test_build_gpu_env_empty() {
        let env = build_gpu_env("");
        assert!(env.is_empty());
    }

    #[test]
    fn test_build_gpu_env_nonexistent() {
        let env = build_gpu_env("card99");
        assert!(env.is_empty());
    }
}
