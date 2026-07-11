use crate::models::GameLaunchConfig;
use crate::models::WineConfig;
use crate::launcher::wine_launch;

const PR_SET_CHILD_SUBREAPER: i32 = 36;

/// Sets this process as a subreaper so orphaned grandchildren
/// (e.g. Wine services, gamescope) get reparented to us and we can
/// waitpid them to prevent zombies. Called once at startup.
pub fn init_subreaper() {
    unsafe { libc::prctl(PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0); }
}

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

    for (k, v) in &launch.env_vars {
        env.push((k.clone(), v.clone()));
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

    let has_wine = wine.map_or(false, |w| w.enabled);
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
        gs_args.extend(command.drain(..));
        *command = gs_args;
    } else {
        let mut final_cmd: Vec<String> = Vec::new();
        final_cmd.extend(extra_prefix);
        final_cmd.extend(command.drain(..));
        *command = final_cmd;
    }

    env
}
