//! Development smoke launcher for the overlay without an Ira game entry.
//!
//! Usage:
//!   ira-overlay-test direct [-- COMMAND...]
//!   ira-overlay-test gamescope [-- COMMAND...]
//!
//! The default command is `vkcube --wsi wayland` on Wayland and
//! `vkcube --wsi xcb` otherwise. Both modes start visible; use Shift+Tab to
//! toggle the overlay.

use std::env;
use std::ffi::CString;
use std::fs;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

use ira_overlay_ipc::{shm_path, MappedShm};

const TEST_GAME_ID: i64 = 9_876_543_210;
const LAYER_NAME: &str = "VK_LAYER_IRA_OVERLAY";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TestMode {
    Direct,
    Gamescope,
}

struct TestConfig {
    mode: TestMode,
    command: Vec<String>,
}

struct Artifacts {
    directory: PathBuf,
    shim: PathBuf,
    vk_layer: PathBuf,
    standalone: PathBuf,
}

fn main() {
    match run() {
        Ok(status) => std::process::exit(status.code().unwrap_or(1)),
        Err(error) => {
            eprintln!("ira-overlay-test: {error}");
            std::process::exit(1);
        }
    }
}

fn run() -> Result<ExitStatus, String> {
    let config = parse_args()?;
    let artifacts = find_artifacts()?;
    let command = config.command;
    let db_id = TEST_GAME_ID + i64::from(std::process::id());
    let shm = create_test_shm(db_id, config.mode == TestMode::Gamescope)?;
    let shm_name = shm_path(db_id);

    eprintln!("ira-overlay-test: mode={:?}", config.mode);
    eprintln!("ira-overlay-test: SHM={shm_name}");
    match config.mode {
        TestMode::Direct | TestMode::Gamescope => {
            eprintln!("ira-overlay-test: overlay starts visible; press Shift+Tab to toggle it")
        }
    }

    let status = match config.mode {
        TestMode::Direct => run_direct(&artifacts, &shm_name, &command)?,
        TestMode::Gamescope => run_gamescope(&artifacts, &shm_name, &command)?,
    };

    unlink_shm(&shm_name);
    drop(shm);
    let _ = fs::remove_dir_all(manifest_directory(&artifacts.directory));
    Ok(status)
}

fn parse_args() -> Result<TestConfig, String> {
    parse_args_from(env::args().skip(1).collect())
}

fn parse_args_from(args: Vec<String>) -> Result<TestConfig, String> {
    let mode = match args.first().map(String::as_str) {
        Some("direct") => TestMode::Direct,
        Some("gamescope") => TestMode::Gamescope,
        _ => return Err(usage()),
    };
    let command = args.into_iter().skip(1).collect::<Vec<_>>();
    let command = if command.first().map(String::as_str) == Some("--") {
        command.into_iter().skip(1).collect()
    } else if command.is_empty() {
        default_command(mode)
    } else {
        command
    };
    if command.is_empty() {
        return Err(usage());
    }
    Ok(TestConfig { mode, command })
}

fn default_command(mode: TestMode) -> Vec<String> {
    let wsi = match mode {
        TestMode::Direct if env::var_os("WAYLAND_DISPLAY").is_some() => "wayland",
        TestMode::Direct | TestMode::Gamescope => "xcb",
    };
    default_command_for_wsi(wsi)
}

fn default_command_for_wsi(wsi: &str) -> Vec<String> {
    vec!["vkcube".to_string(), "--wsi".to_string(), wsi.to_string()]
}

fn usage() -> String {
    "usage: ira-overlay-test <direct|gamescope> [-- COMMAND...]".to_string()
}

fn find_artifacts() -> Result<Artifacts, String> {
    let executable =
        env::current_exe().map_err(|e| format!("cannot find current executable: {e}"))?;
    let directory = executable
        .parent()
        .ok_or_else(|| "current executable has no parent directory".to_string())?
        .to_path_buf();
    let required = |name: &str| {
        let path = directory.join(name);
        path.is_file()
            .then_some(path)
            .ok_or_else(|| format!("missing {name}; build the overlay workspace first"))
    };
    let shim = required("libira_overlay_shim.so")?;
    let vk_layer = required("libira_overlay_vk.so")?;
    let standalone = required("ira-overlay-standalone")?;
    Ok(Artifacts {
        directory,
        shim,
        vk_layer,
        standalone,
    })
}

fn create_test_shm(db_id: i64, visible: bool) -> Result<MappedShm, String> {
    let mut shm = MappedShm::create(db_id)?;
    shm.init_header(db_id);
    {
        let header = shm.header_mut();
        write_bytes(&mut header.game_name, "Ira Overlay Smoke Test");
        write_bytes(&mut header.game_kind, "overlay-test");
        header.total_achievements = 6;
        header.unlocked_achievements = 2;
        header.playtime_seconds = 2 * 3600 + 17 * 60;
        header
            .overlay_visible
            .store(u32::from(visible), std::sync::atomic::Ordering::SeqCst);
    }

    for (index, achievement) in shm.achievements_mut().iter_mut().take(6).enumerate() {
        write_bytes(
            &mut achievement.display_name,
            &format!("Smoke achievement {}", index + 1),
        );
        write_bytes(&mut achievement.description, "This is overlay test data.");
        achievement.earned = u8::from(index < 2);
        achievement.earned_time = 1_700_000_000 + index as u64;
        achievement.global_percent = 25.0 + index as f32;
        achievement.trophy_type = match index {
            0 => b'g',
            1 => b's',
            2 => b'b',
            _ => b'0',
        };
        achievement.hidden = u8::from(index == 5);
    }
    Ok(shm)
}

fn write_bytes(destination: &mut [u8], value: &str) {
    let bytes = value.as_bytes();
    let length = bytes.len().min(destination.len().saturating_sub(1));
    destination[..length].copy_from_slice(&bytes[..length]);
    destination[length] = 0;
}

fn run_direct(
    artifacts: &Artifacts,
    shm_name: &str,
    command: &[String],
) -> Result<ExitStatus, String> {
    let manifest = write_layer_manifest(artifacts)?;
    let mut child = target_command(command)?;
    child
        .env("IRA_OVERLAY_SHM", shm_name)
        .env("IRA_OVERLAY_TEXT_BACKEND", "pango")
        .env("IRA_OVERLAY_START_VISIBLE", "1")
        .env("VK_LAYER_PATH", &manifest)
        .env("VK_INSTANCE_LAYERS", LAYER_NAME)
        .env("LD_PRELOAD", &artifacts.shim);
    child
        .process_group(0)
        .spawn()
        .map_err(|e| format!("failed to start direct target: {e}"))?
        .wait()
        .map_err(|e| format!("failed waiting for direct target: {e}"))
}

fn run_gamescope(
    artifacts: &Artifacts,
    shm_name: &str,
    command: &[String],
) -> Result<ExitStatus, String> {
    let mut gamescope = Command::new("gamescope");
    gamescope.args([
        "--backend",
        "wayland",
        "-w",
        "1280",
        "-h",
        "720",
        "-W",
        "1280",
        "-H",
        "720",
        "--",
        "sh",
        "-c",
        "cleanup() { status=$?; trap - EXIT INT TERM HUP; if [ -n \"${overlay_pid:-}\" ]; then kill \"$overlay_pid\" 2>/dev/null; wait \"$overlay_pid\" 2>/dev/null; fi; exit \"$status\"; }; trap cleanup EXIT; trap 'exit 143' INT TERM HUP; ENABLE_GAMESCOPE_WSI=1 \"$1\" & overlay_pid=$!; shift; \"$@\"",
        "--",
    ]);
    gamescope.arg(&artifacts.standalone).args(command);
    gamescope
        .env("IRA_OVERLAY_SHM", shm_name)
        .env("IRA_OVERLAY_TEXT_BACKEND", "pango");
    gamescope
        .process_group(0)
        .spawn()
        .map_err(|e| format!("failed to start gamescope: {e}"))?
        .wait()
        .map_err(|e| format!("failed waiting for gamescope: {e}"))
}

fn target_command(command: &[String]) -> Result<Command, String> {
    let (program, args) = command
        .split_first()
        .ok_or_else(|| "target command is empty".to_string())?;
    let mut child = Command::new(program);
    child.args(args);
    Ok(child)
}

fn write_layer_manifest(artifacts: &Artifacts) -> Result<PathBuf, String> {
    let directory = manifest_directory(&artifacts.directory);
    fs::create_dir_all(&directory)
        .map_err(|e| format!("failed to create layer manifest directory: {e}"))?;
    let path = directory.join("ira_overlay.json");
    let library = artifacts
        .vk_layer
        .canonicalize()
        .unwrap_or_else(|_| artifacts.vk_layer.clone());
    let content = format!(
        "{{\n  \"file_format_version\": \"1.0.0\",\n  \"layer\": {{\n    \"name\": \"{LAYER_NAME}\",\n    \"type\": \"GLOBAL\",\n    \"api_version\": \"1.3.0\",\n    \"library_path\": \"{}\",\n    \"implementation_version\": \"1\",\n    \"description\": \"Ira game overlay\",\n    \"functions\": {{\n      \"vkNegotiateLoaderLayerInterfaceVersion\": \"vkNegotiateLoaderLayerInterfaceVersion\"\n    }}\n  }}\n}}\n",
        json_string(&library.to_string_lossy())
    );
    fs::write(&path, content).map_err(|e| format!("failed to write layer manifest: {e}"))?;
    Ok(directory)
}

fn json_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn manifest_directory(executable_directory: &Path) -> PathBuf {
    env::temp_dir()
        .join("ira-overlay-test")
        .join(executable_directory.file_name().unwrap_or_default())
}

fn unlink_shm(path: &str) {
    if let Ok(c_path) = CString::new(path) {
        unsafe {
            libc::shm_unlink(c_path.as_ptr());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{default_command, default_command_for_wsi, json_string, parse_args_from, TestMode};

    #[test]
    fn test_default_command_uses_xcb_vkcube() {
        assert_eq!(
            default_command_for_wsi("xcb"),
            ["vkcube", "--wsi", "xcb"]
                .into_iter()
                .map(String::from)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_json_string_escapes_manifest_paths() {
        assert_eq!(json_string("/tmp/a\\b\"c"), "/tmp/a\\\\b\\\"c");
    }

    #[test]
    fn test_parse_args_selects_direct_target() {
        let config = parse_args_from(vec![
            "direct".to_string(),
            "--".to_string(),
            "vkcube".to_string(),
        ])
        .unwrap();
        assert_eq!(config.mode, TestMode::Direct);
        assert_eq!(config.command, ["vkcube"]);
    }

    #[test]
    fn test_parse_args_defaults_to_vkcube() {
        let config = parse_args_from(vec!["gamescope".to_string()]).unwrap();
        assert_eq!(config.mode, TestMode::Gamescope);
        assert_eq!(config.command, ["vkcube", "--wsi", "xcb"]);
    }

    #[test]
    fn test_gamescope_default_uses_xcb() {
        assert_eq!(
            default_command(TestMode::Gamescope),
            ["vkcube", "--wsi", "xcb"]
                .into_iter()
                .map(String::from)
                .collect::<Vec<_>>()
        );
    }
}
