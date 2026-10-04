//! Base launch environment: the sanitized parent environment plus Wine,
//! per-game overrides, GPU selection, shader cache, and performance wrapping.
//!
//! Gamescope composition lives in [`crate::gamescope_env`], overlay
//! injection in [`crate::overlay_env`], and input-broker wrapping in
//! [`crate::input_wrap`]; their public items are re-exported here so the
//! `env_builder::` paths used across the app keep working.

// Re-exported for downstream callers (`ira_launcher::env_builder::…`).
pub use crate::gamescope_env::{
    apply_gamescope_display_policy, apply_performance, pin_flatpak_for_gamescope,
    strip_session_wayland, uses_gamescope, will_use_gamescope, will_wrap_gamescope,
};
pub use crate::input_wrap::{wrap_with_input, wrap_with_input_mode};
pub use crate::overlay_env::{
    add_overlay_env, add_overlay_env_external, add_overlay_env_without_ui,
    maybe_add_daemon_toggle_env, wrap_with_host_overlay,
};

use crate::env_ops::{env_prepend, env_set};
use crate::gamescope_env::session_socket_in_env;
use crate::wine_launch;
use ira_models::{GameLaunchConfig, WineConfig};

/// Splits a `:`-separated path list and drops empty entries plus development
/// directories (cargo, rustup, target dirs), so the dev environment never
/// leaks into launched games.
fn filter_dev_paths(v: &str) -> Vec<&str> {
    v.split(':')
        .filter(|p| {
            !p.is_empty()
                && !p.contains("/.cargo/")
                && !p.contains("/.rustup/")
                && !p.contains("/target/")
        })
        .collect()
}

/// The parent process environment minus build/dev variables that must never
/// reach a launched process (CARGO/RUST toolchain vars), with PATH and
/// LD_LIBRARY_PATH stripped of dev directories. Wine-specific handling is up
/// to callers.
pub fn clean_parent_env() -> Vec<(String, String)> {
    std::env::vars()
        .filter(|(k, _)| {
            // Filter out build/dev environment variables that shouldn't reach the game
            k != "CARGO"
                && !k.starts_with("CARGO_")
                && k != "RUSTUP"
                && !k.starts_with("RUSTUP_")
                && !k.starts_with("RUST_")
                && k != "OUT_DIR"
        })
        .filter(|(k, v)| match k.as_str() {
            // For non-Proton launches: remove if only dev paths
            // (cargo, rustup, target dirs); otherwise keep filtered below.
            "LD_LIBRARY_PATH" | "PATH" => !filter_dev_paths(v).is_empty(),
            _ => true,
        })
        .map(|(k, v)| {
            if k == "LD_LIBRARY_PATH" || k == "PATH" {
                (k, filter_dev_paths(&v).join(":"))
            } else {
                (k, v)
            }
        })
        .collect()
}

pub fn build_env(
    launch: &GameLaunchConfig,
    wine: Option<&WineConfig>,
    wine_exe: &str,
    save_dir: &str,
    game_id: i64,
    _app_id: &str,
    command: &mut Vec<String>,
) -> Vec<(String, String)> {
    let has_wine = wine.is_some_and(|w| w.enabled);
    let is_proton = has_wine
        && (crate::wine_detect::is_proton_version(&wine.unwrap().version)
            || crate::wine_detect::is_proton_binary(wine_exe));

    let mut env = clean_parent_env();
    if is_proton {
        // For Proton/umu, don't pass host LD_LIBRARY_PATH at all.
        // pressure-vessel builds its own STEAM_RUNTIME_LIBRARY_PATH
        // from this — host paths cause library conflicts in the container.
        env.retain(|(k, _)| k != "LD_LIBRARY_PATH");
    }

    if let Some(w) = wine {
        if w.enabled {
            let wine_env = wine_launch::build_wine_env(w, wine_exe);
            // Remove any existing keys that wine_env overrides, then extend
            for (k, _) in &wine_env {
                env.retain(|(ek, _)| ek != k);
            }
            env.extend(wine_env);
        }
    }

    apply_launch_overrides(&mut env, launch);

    if let Some(w) = wine {
        if w.enabled && !w.denuvo_api.is_empty() {
            let denuvo_so = format!(
                "{}/api_emulators/denuvo/{}",
                save_dir,
                w.denuvo_api.trim_start_matches('/')
            );
            if std::path::Path::new(&denuvo_so).is_file() {
                env_prepend(&mut env, "LD_PRELOAD", &denuvo_so);
            } else {
                eprintln!(
                    "Denuvo emulator not found: {} (denuvo_api='{}')",
                    denuvo_so, w.denuvo_api
                );
            }
        }
    }

    let shader_dir = format!("{}/shader_cache/{}", save_dir, game_id);
    let _ = std::fs::create_dir_all(&shader_dir);
    env.push(("__GL_SHADER_DISK_CACHE".to_string(), "1".to_string()));
    env.push(("__GL_SHADER_DISK_CACHE_PATH".to_string(), shader_dir));

    let default_wine = WineConfig::default();
    let wine_cfg = wine.unwrap_or(&default_wine);
    apply_performance(command, &mut env, launch, wine_cfg);
    let in_session = session_socket_in_env(&env);
    apply_gamescope_display_policy(command, &mut env, in_session);

    env
}

/// Applies per-game environment, loader paths, and GPU selection to an
/// already-constructed launch environment. Emulator launches use this shared
/// path too; they do not go through `build_env`.
pub fn apply_launch_overrides(env: &mut Vec<(String, String)>, launch: &GameLaunchConfig) {
    for (k, v) in &launch.env_vars {
        env_set(env, k, v);
    }

    if !launch.ld_preload.is_empty() {
        env_prepend(env, "LD_PRELOAD", &launch.ld_preload);
    }
    if !launch.ld_library_path.is_empty() {
        env_prepend(env, "LD_LIBRARY_PATH", &launch.ld_library_path);
    }

    if !launch.gpu.is_empty() {
        for (k, v) in crate::gpu::build_gpu_env(&launch.gpu) {
            env_set(env, &k, &v);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_dev_paths_strips_cargo_rustup_target() {
        let filtered = filter_dev_paths(
            "/usr/bin:/home/ayla/.cargo/bin:/home/ayla/.rustup/toolchains/nightly-x86_64/bin:/data/build/target/debug:/usr/local/bin",
        );
        assert_eq!(filtered, vec!["/usr/bin", "/usr/local/bin"]);
    }

    #[test]
    fn test_filter_dev_paths_empty_and_dev_only() {
        assert!(filter_dev_paths("").is_empty());
        assert!(filter_dev_paths("/home/ayla/.cargo/bin").is_empty());
        assert!(filter_dev_paths(":/home/ayla/.rustup/bin/:").is_empty());
    }

    #[test]
    fn test_apply_launch_overrides_merges_user_environment() {
        let launch = GameLaunchConfig {
            env_vars: vec![("GAME_MODE".to_string(), "test".to_string())],
            ld_preload: "/configured/preload.so".to_string(),
            ld_library_path: "/configured/lib".to_string(),
            ..Default::default()
        };
        let mut env = vec![
            ("GAME_MODE".to_string(), "old".to_string()),
            ("LD_PRELOAD".to_string(), "/existing/preload.so".to_string()),
            ("LD_LIBRARY_PATH".to_string(), "/existing/lib".to_string()),
        ];

        apply_launch_overrides(&mut env, &launch);

        assert!(env.contains(&("GAME_MODE".to_string(), "test".to_string())));
        assert!(env.contains(&(
            "LD_PRELOAD".to_string(),
            "/configured/preload.so:/existing/preload.so".to_string()
        )));
        assert!(env.contains(&(
            "LD_LIBRARY_PATH".to_string(),
            "/configured/lib:/existing/lib".to_string()
        )));
    }
}
