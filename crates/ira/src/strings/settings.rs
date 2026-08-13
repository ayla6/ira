pub const LOADING: &str = "Loading Settings";
pub const LOADING_DESCRIPTION: &str = "Checking installed emulators and controller profiles";
pub const LOADING_EMULATOR: &str = "Loading emulator settings";

pub const GENERAL: &str = "General";
pub const OVERLAY: &str = "Overlay";
pub const CONTROLLER: &str = "Controller";
pub const GAME_SYSTEM: &str = "Game system";
pub const PC_GAMES: &str = "PC games";
pub const STEAM: &str = "Steam";
pub const API_EMULATORS: &str = "API emulators";
pub const LUTRIS_MIGRATION: &str = "Lutris migration";
pub const WINE: &str = "Wine";
pub const PROFILES: &str = "Profiles";
pub const EMULATION: &str = "Emulation";
pub const RETROACHIEVEMENTS: &str = "RetroAchievements";
pub const ROM_LIBRARY: &str = "ROM library";
pub const EMPTY_PLATFORMS: &str = "Empty platforms";

pub const IN_GAME_OVERLAY: &str = "In-game overlay";
pub const OVERLAY_DESCRIPTION: &str = "Achievements, screenshots, and recording";
pub const GAMESCOPE: &str = "Gamescope";
pub const GAMESCOPE_DESCRIPTION: &str = "Valve Gamescope compositor";
pub const EMULATOR: &str = "Emulator";
pub const VERSION: &str = "Version";
pub const INSTALL_DIRECTORIES: &str = "Install directories";
pub const INSTALL_DIRECTORY: &str = "Install directory";
pub const PERFORMANCE: &str = "Performance";
pub const GAMEMODE: &str = "Gamemode";
pub const GAMEMODE_DESCRIPTION: &str = "Feral Interactive GameMode";
pub const MANGOHUD: &str = "MangoHud";
pub const MANGOHUD_DESCRIPTION: &str = "Performance overlay";

pub const CENTRALIZE_SAVES: &str = "Centralize game saves";
pub const CENTRALIZE_SAVES_DESCRIPTION: &str =
    "Symlink save data to a central location so it persists across Wine prefix resets";
pub const LANGUAGE_PREFERENCES: &str = "Language preferences";
pub const LANGUAGE_PREFERENCES_DESCRIPTION: &str =
    "When a game is added, the first supported language from this list is used for the emulator config";
pub const GRAPHICS: &str = "Graphics";
pub const GPU: &str = "GPU";
pub const GPU_DESCRIPTION: &str = "Graphics card to use for rendering by default";

pub const LUTRIS_INSTALLATION: &str = "Lutris installation";
pub const LUTRIS_DATA_DIRECTORY: &str = "Lutris data directory";
pub const LUTRIS_NOT_FOUND: &str = "Lutris not found";
pub const MIGRATION: &str = "Migration";
pub const IMPORT_LUTRIS_GAMES: &str = "Import Lutris games";
pub const IMPORT_LUTRIS_DESCRIPTION: &str =
    "Reads each Lutris game's config and creates a game entry with wine settings";
pub const IMPORT_ALL: &str = "Import all";
pub const MIGRATE: &str = "Migrate";

pub const ENABLE_STEAM: &str = "Enable Steam integration";
pub const ENABLE_STEAM_DESCRIPTION: &str =
    "Scan your Steam library for installed games and achievements";
pub const STEAM_INSTALLATION: &str = "Steam installation";
pub const STEAM_DIRECTORY: &str = "Steam directory";
pub const STEAM_NOT_FOUND: &str = "Steam not found";
pub const STEAM_USER_IDS: &str = "Steam user IDs";
pub const NONE_FOUND: &str = "None found";

pub const DEFAULT_GAME_FOLDER: &str = "Default game folder";
pub const GAME_FOLDER: &str = "Game folder";
pub const SELECT_DEFAULT_GAME_FOLDER: &str = "Select default game folder";
pub const BASE_ROM_FOLDER: &str = "Base ROM folder";
pub const SELECT_BASE_ROM_FOLDER: &str = "Select base ROM folder";
pub const ROM_FOLDER_DESCRIPTION: &str =
    "ROMs are stored in one folder with a subfolder for each system, such as gba, psx, and ps2.";

pub const ENABLE_RETROACHIEVEMENTS: &str = "Enable RetroAchievements";
pub const ENABLE_RETROACHIEVEMENTS_DESCRIPTION: &str =
    "Fetch achievements for matched retro games from retroachievements.org";
pub const ACCOUNT: &str = "Account";
pub const USERNAME: &str = "Username";
pub const PASSWORD: &str = "Password";

pub const API_EMULATOR_FILES: &str = "API emulator files";
pub const API_EMULATOR_FILES_DESCRIPTION: &str = "Drop emulator files into the structure below";
pub const DIRECTORY: &str = "Directory";
pub const OPEN: &str = "Open";
pub const DEFAULT_VERSION: &str = "Default version";
pub const DEFAULT_VERSION_DESCRIPTION: &str =
    "Version to use when installing API emulators on games";
pub const EMULATOR_VERSION: &str = "Emulator version";
pub const EMULATOR_VERSION_DESCRIPTION: &str = "Default version directory to use when installing";
pub const NO_VERSIONS_INSTALLED: &str = "(no versions installed)";

pub const ENABLE_OVERLAY: &str = "Enable in-game overlay";
pub const ENABLE_OVERLAY_DESCRIPTION: &str =
    "Shows achievements, screenshots, and recording during gameplay (Vulkan games only)";
pub const RECORDING: &str = "Recording";
pub const VIDEO_ENCODER: &str = "Video encoder";
pub const VIDEO_ENCODER_DESCRIPTION: &str =
    "Auto detects the best available encoder. Use Software if your GPU lacks hardware encoding.";
pub const RECORDING_QUALITY: &str = "Recording quality";
pub const HOTKEYS: &str = "Hotkeys";
pub const HOTKEYS_DESCRIPTION: &str = "Keyboard and gamepad bindings. Click to set.";
