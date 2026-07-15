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

pub fn build_env(
    launch: &GameLaunchConfig,
    wine: Option<&WineConfig>,
    wine_exe: &str,
    save_dir: &str,
    game_id: i64,
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
