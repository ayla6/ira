use ira_models::GameLaunchConfig;
use ira_models::WineConfig;
use crate::wine_launch;

fn has_exec(name: &str) -> bool {
    std::env::var_os("PATH")
        .and_then(|p| {
            std::env::split_paths(&p).find(|d| d.join(name).is_file())
        })
        .is_some()
}

/// Returns `(layer_json_dir, shim_so_path)` if the overlay files are found.
/// In development, generates a temporary JSON manifest with the correct
/// `library_path` (the static JSON points to release/, which is wrong in debug).
fn overlay_paths() -> Option<(String, String)> {
    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;

    // Development: .so files are in target/debug or target/release alongside the exe.
    let dev_shim = exe_dir.join("libira_overlay_shim.so");
    let dev_vk = exe_dir.join("libira_overlay_vk.so");
    if dev_shim.is_file() && dev_vk.is_file() {
        // Generate a temporary JSON manifest with the correct absolute library_path.
        // The static JSON in crates/overlay-vk/ points to release/, which is wrong
        // for debug builds. Writing our own avoids modifying the source file.
        let tmp_dir = std::env::temp_dir().join("ira_overlay");
        let _ = std::fs::create_dir_all(&tmp_dir);
        let json_path = tmp_dir.join("ira_overlay.json");
        let vk_abs = dev_vk.canonicalize().unwrap_or(dev_vk.clone());
        let json_content = format!(
            r#"{{
    "file_format_version": "1.0.0",
    "layer": {{
        "name": "VK_LAYER_IRA_OVERLAY",
        "type": "GLOBAL",
        "api_version": "1.2.0",
        "library_path": "{}",
        "implementation_version": "1",
        "description": "Ira game overlay",
        "functions": {{
            "vkNegotiateLoaderLayerInterfaceVersion": "vkNegotiateLoaderLayerInterfaceVersion"
        }}
    }}
}}"#,
            vk_abs.to_string_lossy()
        );
        if std::fs::write(&json_path, json_content).is_ok() {
            return Some((
                tmp_dir.to_string_lossy().into(),
                dev_shim.to_string_lossy().into(),
            ));
        }
    }

    // Release: files installed in an overlay/ subdirectory alongside the exe.
    let overlay_dir = exe_dir.join("overlay");
    let rel_shim = overlay_dir.join("libira_overlay_shim.so");
    let rel_json = overlay_dir.join("ira_overlay.json");
    if rel_shim.is_file() && rel_json.is_file() {
        return Some((
            overlay_dir.to_string_lossy().into(),
            rel_shim.to_string_lossy().into(),
        ));
    }

    None
}

pub fn build_env(
    launch: &GameLaunchConfig,
    wine: Option<&WineConfig>,
    wine_exe: &str,
    save_dir: &str,
    game_id: i64,
    app_id: &str,
    command: &mut Vec<String>,
) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = std::env::vars().collect();

    let has_wine = wine.is_some_and(|w| w.enabled);

    if !has_wine {
        for (k, v) in &launch.env_vars {
            env.retain(|(ek, _)| ek != k);
            env.push((k.clone(), v.clone()));
        }

        if !launch.ld_preload.is_empty() {
            let existing = env.iter().find(|(k, _)| k == "LD_PRELOAD").map(|(_, v)| v.clone());
            let merged = match existing {
                Some(prev) if !prev.is_empty() => format!("{}:{}", launch.ld_preload, prev),
                _ => launch.ld_preload.clone(),
            };
            env.retain(|(k, _)| k != "LD_PRELOAD");
            env.push(("LD_PRELOAD".to_string(), merged));
        }
        if !launch.ld_library_path.is_empty() {
            let existing = env.iter().find(|(k, _)| k == "LD_LIBRARY_PATH").map(|(_, v)| v.clone());
            let merged = match existing {
                Some(prev) if !prev.is_empty() => format!("{}:{}", launch.ld_library_path, prev),
                _ => launch.ld_library_path.clone(),
            };
            env.retain(|(k, _)| k != "LD_LIBRARY_PATH");
            env.push(("LD_LIBRARY_PATH".to_string(), merged));
        }
    }

    if let Some(w) = wine {
        if w.enabled {
            let wine_env = wine_launch::build_wine_env(w, wine_exe);
            env.extend(wine_env);
            if w.umu_enabled {
                env.push(("PROTON_VERB".to_string(), "waitforexitandrun".to_string()));
                if !app_id.is_empty() {
                    env.push(("GAMEID".to_string(), app_id.to_string()));
                }
            }
        }
    }

    let shader_dir = format!("{}/shader_cache/{}", save_dir, game_id);
    let _ = std::fs::create_dir_all(&shader_dir);
    env.push(("__GL_SHADER_DISK_CACHE".to_string(), "1".to_string()));
    env.push(("__GL_SHADER_DISK_CACHE_PATH".to_string(), shader_dir));

    let has_wine = wine.is_some_and(|w| w.enabled);
    let mut extra_prefix: Vec<String> = Vec::new();

    if has_wine && wine.unwrap().gamemode && has_exec("gamemoderun") {
        extra_prefix.push("gamemoderun".to_string());
    }
    if has_wine && wine.unwrap().mangohud && has_exec("mangohud") {
        extra_prefix.push("mangohud".to_string());
        env.push(("MANGOHUD".to_string(), "1".to_string()));
        env.push(("MANGOHUD_DLSYM".to_string(), "1".to_string()));
    }

    if has_wine && wine.unwrap().gamescope && has_exec("gamescope") {
        let mut gs_args = vec!["gamescope".to_string()];
        if !wine.unwrap().gamescope_flags.is_empty() {
            if let Some(flags) = shlex::split(&wine.unwrap().gamescope_flags) {
                gs_args.extend(flags);
            }
        }
        gs_args.push("--".to_string());
        gs_args.extend(extra_prefix);
        gs_args.append(command);
        *command = gs_args;
    } else {
        let mut final_cmd: Vec<String> = Vec::new();
        final_cmd.extend(extra_prefix);
        final_cmd.append(command);
        *command = final_cmd;
    }

    env
}

/// Adds overlay env vars (VK_LAYER_PATH, VK_INSTANCE_LAYERS, LD_PRELOAD, IRA_OVERLAY_SHM)
/// to an existing env list. Call this after `build_env` when overlay is enabled.
/// Does nothing if the overlay files are not found on disk.
pub fn add_overlay_env(env: &mut Vec<(String, String)>, overlay_shm: Option<&str>) {
    let Some((layer_dir, shim_path)) = overlay_paths() else {
        eprintln!("ira-overlay: enabled but files not found — skipping injection");
        return;
    };

    env.retain(|(k, _)| k != "VK_LAYER_PATH");
    env.push(("VK_LAYER_PATH".to_string(), layer_dir));
    env.retain(|(k, _)| k != "VK_INSTANCE_LAYERS");
    env.push(("VK_INSTANCE_LAYERS".to_string(), "VK_LAYER_IRA_OVERLAY".to_string()));

    let existing = env.iter().find(|(k, _)| k == "LD_PRELOAD").map(|(_, v)| v.clone());
    let merged = match existing {
        Some(prev) if !prev.is_empty() => format!("{}:{}", shim_path, prev),
        _ => shim_path,
    };
    env.retain(|(k, _)| k != "LD_PRELOAD");
    env.push(("LD_PRELOAD".to_string(), merged));

    if let Some(shm) = overlay_shm {
        env.retain(|(k, _)| k != "IRA_OVERLAY_SHM");
        env.push(("IRA_OVERLAY_SHM".to_string(), shm.to_string()));
    }
}
