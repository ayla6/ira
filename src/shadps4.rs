use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::AppMessage;
use crate::parser::{Game, MergedAchievement};
use notify::{Config as NotifyConfig, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

/// shadPS4 user data directory.
/// Linux: ~/.local/share/shadPS4/
pub fn shadps4_user_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".local").join("share").join("shadPS4")
}

/// Path to play_time.txt
pub fn play_time_path() -> PathBuf {
    shadps4_user_dir().join("play_time.txt")
}

/// Path to the trophy directory
pub fn trophy_dir(npwr_id: &str) -> PathBuf {
    shadps4_user_dir().join("trophy").join(npwr_id)
}

/// Path to the user trophy unlock-state XML
pub fn user_trophy_path(npwr_id: &str) -> PathBuf {
    shadps4_user_dir()
        .join("home")
        .join("1000")
        .join("trophy")
        .join(format!("{}.xml", npwr_id))
}

// ============== PSF (param.sfo) parsing ==============

const PSF_MAGIC: u32 = 0x00505346;

#[derive(Debug, Clone)]
pub enum PsfValue {
    Text(String),
    Integer(i32),
    Binary(Vec<u8>),
}

#[repr(C)]
struct PsfHeader {
    magic: [u8; 4],
    version: [u8; 4],
    key_table_offset: [u8; 4],
    data_table_offset: [u8; 4],
    index_entries: [u8; 4],
}

#[repr(C)]
struct PsfEntry {
    key_offset: [u8; 2],
    param_fmt: [u8; 2],
    param_len: [u8; 4],
    param_max_len: [u8; 4],
    data_offset: [u8; 4],
}

fn read_u32_le(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}
fn read_u16_le(b: &[u8]) -> u16 {
    u16::from_le_bytes([b[0], b[1]])
}
fn read_u16_be(b: &[u8]) -> u16 {
    u16::from_be_bytes([b[0], b[1]])
}

/// Parse a param.sfo file, returning a map of key → value.
pub fn parse_psf(path: &Path) -> Result<HashMap<String, PsfValue>, String> {
    let data = std::fs::read(path).map_err(|e| format!("read {}: {}", path.display(), e))?;
    if data.len() < 20 {
        return Err("param.sfo too short".into());
    }

    let magic = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
    if magic != PSF_MAGIC {
        return Err(format!("bad PSF magic: 0x{:08X}", magic));
    }

    let key_table_offset = read_u32_le(&data[8..12]) as usize;
    let data_table_offset = read_u32_le(&data[12..16]) as usize;
    let num_entries = read_u32_le(&data[16..20]) as usize;

    let mut result = HashMap::new();

    for i in 0..num_entries {
        let off = 20 + i * 16;
        if off + 16 > data.len() {
            break;
        }
        let key_off = read_u16_le(&data[off..off + 2]) as usize;
        let param_fmt = read_u16_le(&data[off + 2..off + 4]);
        let param_len = read_u32_le(&data[off + 4..off + 8]) as usize;
        let data_off = read_u32_le(&data[off + 12..off + 16]) as usize;

        // Read key string (null-terminated)
        let key_start = key_table_offset + key_off;
        let key_end = data[key_start..]
            .iter()
            .position(|&b| b == 0)
            .map(|p| key_start + p)
            .unwrap_or(data.len());
        let key = String::from_utf8_lossy(&data[key_start..key_end]).to_string();

        // Read data
        let data_start = data_table_offset + data_off;
        let data_end = data_start + param_len;

        let value = match param_fmt {
            0x0204 => {
                // Text (UTF-8, null-terminated)
                let s = if data_end <= data.len() {
                    let slice = &data[data_start..data_end];
                    let nul = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());
                    String::from_utf8_lossy(&slice[..nul]).to_string()
                } else {
                    String::new()
                };
                PsfValue::Text(s)
            }
            0x0404 => {
                // Integer (i32 LE)
                let val = if data_end <= data.len() {
                    i32::from_le_bytes([
                        data[data_start],
                        data[data_start + 1],
                        data[data_start + 2],
                        data[data_start + 3],
                    ])
                } else {
                    0
                };
                PsfValue::Integer(val)
            }
            _ => {
                // Binary
                let bytes = if data_end <= data.len() {
                    data[data_start..data_end].to_vec()
                } else {
                    Vec::new()
                };
                PsfValue::Binary(bytes)
            }
        };

        result.insert(key, value);
    }

    Ok(result)
}

/// Extract TITLE and TITLE_ID from a parsed PSF.
pub fn psf_get_title(psf: &HashMap<String, PsfValue>) -> String {
    match psf.get("TITLE") {
        Some(PsfValue::Text(s)) => s.clone(),
        _ => String::new(),
    }
}

pub fn psf_get_title_id(psf: &HashMap<String, PsfValue>) -> String {
    match psf.get("TITLE_ID") {
        Some(PsfValue::Text(s)) => s.clone(),
        _ => String::new(),
    }
}

/// Recursively scan a directory for game folders (containing sce_sys/param.sfo).
/// Max depth: 5 levels. Skips dirs ending in -UPDATE or -patch.
pub fn scan_dir_for_test(path: &Path, results: &mut Vec<PathBuf>) {
    scan_dir(path, results, 0);
}

// ============== npbind.dat parsing ==============

const NPBIND_MAGIC: u32 = 0xD294A018;

/// Extract NPCommId (e.g. "NPWR11866_00") from npbind.dat.
pub fn parse_npbind(path: &Path) -> Result<Vec<String>, String> {
    let data = std::fs::read(path).map_err(|e| format!("read {}: {}", path.display(), e))?;
    if data.len() < 0x88 {
        return Err("npbind.dat too short".into());
    }

    let magic = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
    if magic != NPBIND_MAGIC {
        return Err(format!("bad npbind magic: 0x{:08X}", magic));
    }

    let num_entries = u64::from_be_bytes([
        data[0x18], data[0x19], data[0x1a], data[0x1b],
        data[0x1c], data[0x1d], data[0x1e], data[0x1f],
    ]) as usize;
    let mut offset = 0x80usize;
    let mut result = Vec::new();

    for _ in 0..num_entries {
        if offset + 8 > data.len() {
            break;
        }
        // Read 4 TLV entries
        for _ in 0..4 {
            if offset + 4 > data.len() {
                break;
            }
            let tlv_type = u16::from_be_bytes([data[offset], data[offset + 1]]);
            let tlv_size = u16::from_be_bytes([data[offset + 2], data[offset + 3]]) as usize;
            offset += 4;

            if tlv_type == 0x0010 {
                // NPCommId
                if offset + tlv_size <= data.len() {
                    let slice = &data[offset..offset + tlv_size];
                    let nul = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());
                    let id = String::from_utf8_lossy(&slice[..nul]).to_string();
                    if !id.is_empty() {
                        result.push(id);
                    }
                }
            }
            offset += tlv_size;
        }
        // Skip 0x98 bytes of padding
        offset += 0x98;
    }

    Ok(result)
}

// ============== Game discovery ==============

/// Read install_dirs from shadPS4 config.json
pub fn read_install_dirs() -> Vec<PathBuf> {
    let config_path = shadps4_user_dir().join("config.json");
    let data = match std::fs::read_to_string(&config_path) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };
    let json: serde_json::Value = match serde_json::from_str(&data) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let mut dirs = Vec::new();
    if let Some(install_dirs) = json.get("General").and_then(|g| g.get("install_dirs")).and_then(|d| d.as_array()) {
        for dir in install_dirs {
            let enabled = dir.get("enabled").and_then(|e| e.as_bool()).unwrap_or(false);
            if !enabled {
                continue;
            }
            if let Some(path) = dir.get("path").and_then(|p| p.as_str()) {
                dirs.push(PathBuf::from(path));
            }
        }
    }
    dirs
}

/// A discovered shadPS4 game.
pub struct ShadPS4Game {
    pub serial: String,
    pub npwr_id: String,
    pub title: String,
    pub game_path: PathBuf,
}

/// Recursively scan a directory for game folders (containing sce_sys/param.sfo).
/// Max depth: 5 levels. Skips dirs ending in -UPDATE or -patch.
fn scan_dir(path: &Path, results: &mut Vec<PathBuf>, depth: u32) {
    if depth > 5 {
        return;
    }
    let param_sfo = path.join("sce_sys").join("param.sfo");
    if param_sfo.is_file() {
        results.push(path.to_path_buf());
        return;
    }
    let entries = match std::fs::read_dir(path) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.ends_with("-UPDATE") || name_str.ends_with("-patch") {
            continue;
        }
        scan_dir(&entry.path(), results, depth + 1);
    }
}

/// Discover all installed shadPS4 games.
pub fn discover_games() -> Vec<ShadPS4Game> {
    let install_dirs = read_install_dirs();
    let mut game_dirs = Vec::new();
    for dir in &install_dirs {
        scan_dir(dir, &mut game_dirs, 0);
    }

    let mut games = Vec::new();
    for game_path in game_dirs {
        let param_sfo = game_path.join("sce_sys").join("param.sfo");
        let psf = match parse_psf(&param_sfo) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("shadPS4: skip {}: {}", game_path.display(), e);
                continue;
            }
        };

        let title = psf_get_title(&psf);
        let serial = psf_get_title_id(&psf);
        if serial.is_empty() {
            continue;
        }

        // Get NPWR ID from npbind.dat
        let npbind_path = game_path.join("sce_sys").join("npbind.dat");
        let npwr_id = match parse_npbind(&npbind_path) {
            Ok(ids) => ids.into_iter().next().unwrap_or_default(),
            Err(_) => {
                // Fallback: try strings extraction
                let npwr = std::fs::read(&npbind_path)
                    .ok()
                    .and_then(|data| {
                        String::from_utf8_lossy(&data)
                            .lines()
                            .find(|l| l.starts_with("NPWR"))
                            .map(|s| s.trim().to_string())
                    })
                    .unwrap_or_default();
                if npwr.is_empty() {
                    eprintln!("shadPS4: no NPWR ID for {}", serial);
                    continue;
                }
                npwr
            }
        };

        games.push(ShadPS4Game {
            serial,
            npwr_id,
            title,
            game_path,
        });
    }

    games
}

// ============== Playtime parsing ==============

/// Read play_time.txt, returning CUSA_ID → playtime string ("H:MM:SS").
pub fn read_play_times() -> HashMap<String, String> {
    let mut result = HashMap::new();
    let path = play_time_path();
    let data = match std::fs::read_to_string(&path) {
        Ok(d) => d,
        Err(_) => return result,
    };
    for line in data.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.splitn(2, ' ').collect();
        if parts.len() == 2 {
            result.insert(parts[0].to_string(), parts[1].to_string());
        }
    }
    result
}

/// Parse "H:MM:SS" → hours as f64
pub fn parse_playtime(time_str: &str) -> f64 {
    let parts: Vec<&str> = time_str.split(':').collect();
    match parts.len() {
        3 => {
            let h: f64 = parts[0].parse().unwrap_or(0.0);
            let m: f64 = parts[1].parse().unwrap_or(0.0);
            let s: f64 = parts[2].parse().unwrap_or(0.0);
            h + m / 60.0 + s / 3600.0
        }
        2 => {
            let m: f64 = parts[0].parse().unwrap_or(0.0);
            let s: f64 = parts[1].parse().unwrap_or(0.0);
            m / 60.0 + s / 3600.0
        }
        _ => 0.0,
    }
}

// ============== Trophy XML parsing ==============

/// A trophy definition from TROP.XML
#[derive(Debug, Clone)]
pub struct TrophyDef {
    pub id: String,
    pub name: String,
    pub detail: String,
    pub ttype: char,
    pub hidden: bool,
}

/// Parse TROP.XML to get trophy definitions (name, detail, type, hidden).
pub fn parse_trop_xml_pub(path: &Path) -> Vec<TrophyDef> {
    parse_trop_xml(path)
}

/// Parse user trophy XML to get unlock states.
/// Returns map of trophy_id → (earned, timestamp)
pub fn parse_user_trophies_pub(path: &Path) -> HashMap<String, (bool, i64)> {
    parse_user_trophies(path)
}

/// Parse TROP.XML to get trophy definitions (name, detail, type, hidden).
fn parse_trop_xml(path: &Path) -> Vec<TrophyDef> {
    let data = match std::fs::read_to_string(path) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };

    let mut trophies = Vec::new();
    let mut current_id = String::new();
    let mut current_name = String::new();
    let mut current_detail = String::new();
    let mut current_ttype = 'B';
    let mut current_hidden = false;
    let mut in_trophy = false;
    let mut in_name = false;
    let mut in_detail = false;
    let mut tag_buf = String::new();

    let mut chars = data.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '<' {
            // Start of a tag
            tag_buf.clear();
            let mut closing = false;
            if chars.peek() == Some(&'/') {
                chars.next();
                closing = true;
            }
            while let Some(&ch) = chars.peek() {
                if ch == '>' {
                    chars.next();
                    break;
                }
                tag_buf.push(ch);
                chars.next();
            }

            let tag = tag_buf.trim();
            let tag_name = tag.split_whitespace().next().unwrap_or("");

            if closing {
                if tag_name == "trophy" && in_trophy {
                    trophies.push(TrophyDef {
                        id: current_id.clone(),
                        name: current_name.clone(),
                        detail: current_detail.clone(),
                        ttype: current_ttype,
                        hidden: current_hidden,
                    });
                    in_trophy = false;
                } else if tag_name == "name" {
                    in_name = false;
                } else if tag_name == "detail" {
                    in_detail = false;
                }
            } else {
                if tag_name == "trophy" {
                    in_trophy = true;
                    current_name.clear();
                    current_detail.clear();
                    current_ttype = 'B';
                    current_hidden = false;
                    // Parse attributes
                    for attr in tag.split_whitespace() {
                        if let Some(eq_pos) = attr.find('=') {
                            let key = &attr[..eq_pos];
                            let val = attr[eq_pos + 1..].trim_matches('"');
                            match key {
                                "id" => current_id = val.to_string(),
                                "ttype" => {
                                    current_ttype = val.chars().next().unwrap_or('B');
                                }
                                "hidden" => current_hidden = val == "yes",
                                _ => {}
                            }
                        }
                    }
                } else if tag_name == "name" {
                    in_name = true;
                    current_name.clear();
                } else if tag_name == "detail" {
                    in_detail = true;
                    current_detail.clear();
                }
            }
        } else if in_name {
            current_name.push(c);
        } else if in_detail {
            current_detail.push(c);
        }
    }

    trophies
}

/// Parse user trophy XML to get unlock states.
/// Returns map of trophy_id → (earned, timestamp)
fn parse_user_trophies(path: &Path) -> HashMap<String, (bool, i64)> {
    let mut result = HashMap::new();
    let data = match std::fs::read_to_string(path) {
        Ok(d) => d,
        Err(_) => return result,
    };

    let mut chars = data.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '<' {
            let mut tag = String::new();
            let mut closing = false;
            if chars.peek() == Some(&'/') {
                chars.next();
                closing = true;
            }
            while let Some(&ch) = chars.peek() {
                if ch == '>' {
                    chars.next();
                    break;
                }
                tag.push(ch);
                chars.next();
            }

            if !closing && tag.trim().starts_with("trophy") {
                let mut id = String::new();
                let mut earned = false;
                let mut timestamp: i64 = 0;

                for attr in tag.split_whitespace() {
                    if let Some(eq_pos) = attr.find('=') {
                        let key = &attr[..eq_pos];
                        let val = attr[eq_pos + 1..].trim_matches('"');
                        match key {
                            "id" => id = val.to_string(),
                            "unlockstate" => earned = val == "true",
                            "timestamp" => timestamp = val.parse().unwrap_or(0),
                            _ => {}
                        }
                    }
                }

                if !id.is_empty() {
                    result.insert(id, (earned, timestamp));
                }
            }
        }
    }

    result
}

// ============== Game loading ==============

/// Map PS4 trophy type to rarity string
fn trophy_type_rarity(ttype: char) -> &'static str {
    match ttype {
        'P' => "Platinum",
        'G' => "Gold",
        'S' => "Silver",
        'B' => "Bronze",
        _ => "Bronze",
    }
}

/// Load a shadPS4 game as a Game struct with achievements.
pub fn load_shadps4_game(
    shad: &ShadPS4Game,
    db_id: i64,
    title: &str,
    hidden: bool,
    logo_position: &str,
    logo_size: i32,
    sort_title: &str,
    sgdb_id: &str,
    save_dir: &str,
) -> Game {
    let npwr_id = &shad.npwr_id;
    let serial = &shad.serial;

    // Trophy definitions
    let trop_xml = trophy_dir(npwr_id).join("Xml").join("TROP.XML");
    let defs = parse_trop_xml(&trop_xml);

    // User unlock state
    let user_xml = user_trophy_path(npwr_id);
    let unlock_states = parse_user_trophies(&user_xml);

    // Trophy icons dir
    let icons_dir = trophy_dir(npwr_id).join("Icons");

    // Playtime
    let play_times = read_play_times();
    let playtime_str = play_times.get(serial).cloned().unwrap_or_default();
    let playtime = parse_playtime(&playtime_str);

    // Build game
    let mut game = Game {
        app_id: npwr_id.clone(),
        kind: "ps4".to_string(),
        platform_id: serial.clone(),
        db_id,
        name: if title.is_empty() {
            shad.title.clone()
        } else {
            title.to_string()
        },
        icon_path: String::new(),
        hero_image_path: String::new(),
        grid_path: String::new(),
        header_path: String::new(),
        logo_path: String::new(),
        achievements: Vec::new(),
        earned_count: 0,
        total_count: 0,
        hidden,
        lutris_id: serial_to_lutris_id(serial),
        slug: serial.clone(),
        playtime,
        lastplayed: 0,
        logo_position: logo_position.to_string(),
        logo_size,
        lutris_name: shad.title.clone(),
        manual_unmatch: false,
        sort_title: sort_title.to_string(),
        game_path: shad.game_path.to_string_lossy().into_owned(),
        sgdb_id: sgdb_id.to_string(),
    };

    // Image paths — use SGDB dir if sgdb_id is set, otherwise data/ps4/{NPWR_ID}/
    let image_dir = if !sgdb_id.is_empty() {
        Path::new(save_dir).join("data").join("steamgriddb").join(sgdb_id)
    } else {
        Path::new(save_dir).join("data").join("ps4").join(npwr_id)
    };

    let icon_png = image_dir.join("icon.png");
    if icon_png.is_file() {
        game.icon_path = icon_png.to_string_lossy().into_owned();
    } else {
        // Fallback: game's sce_sys/icon0.png
        let default_icon = shad.game_path.join("sce_sys").join("icon0.png");
        if default_icon.is_file() {
            game.icon_path = default_icon.to_string_lossy().into_owned();
        }
    }

    let hero = image_dir.join("library_hero.jpg");
    if hero.is_file() {
        game.hero_image_path = hero.to_string_lossy().into_owned();
    }
    let grid = image_dir.join("library_600x900.jpg");
    if grid.is_file() {
        game.grid_path = grid.to_string_lossy().into_owned();
    }
    let header = image_dir.join("header.jpg");
    if header.is_file() {
        game.header_path = header.to_string_lossy().into_owned();
    }
    let logo = image_dir.join("logo.png");
    if logo.is_file() {
        game.logo_path = logo.to_string_lossy().into_owned();
    }

    // Build achievements
    for def in &defs {
        let (earned, timestamp) = unlock_states
            .get(&def.id)
            .cloned()
            .unwrap_or((false, 0));

        // Icon path: TROP000.PNG, TROP001.PNG, etc.
        let icon_name = format!("TROP{:03}.PNG", def.id.parse::<u32>().unwrap_or(0));
        let icon_path = icons_dir.join(&icon_name);
        let icon_str = if icon_path.is_file() {
            icon_path.to_string_lossy().into_owned()
        } else {
            String::new()
        };

        let ach = MergedAchievement {
            name: format!("trophy_{}", def.id),
            display_name: def.name.clone(),
            description: def.detail.clone(),
            hidden: def.hidden,
            earned,
            earned_time: timestamp,
            icon_path: icon_str.clone(),
            icon_gray_path: icon_str, // PS4 doesn't have gray icons — reuse same
            global_percent: 0.0,
        };

        game.total_count += 1;
        if earned {
            game.earned_count += 1;
        }
        game.achievements.push(ach);
    }

    game
}

/// Generate a stable synthetic lutris_id from a CUSA ID.
/// Uses negative range to avoid collision with real Lutris IDs.
pub fn serial_to_lutris_id(serial: &str) -> i64 {
    let hash = serial.bytes().fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));
    -((hash % 1_000_000) as i64 + 2_000_000)
}

/// Check if a lutris_id is a shadPS4 synthetic ID.
pub fn is_shadps4_id(lutris_id: i64) -> bool {
    lutris_id <= -2_000_000
}

// ============== Play time watcher ==============

/// Watches play_time.txt for changes and sends a message.
/// Uses inotify — zero CPU when idle.
pub struct ShadPS4Watcher {
    _watcher: std::sync::Arc<std::sync::Mutex<RecommendedWatcher>>,
}

impl ShadPS4Watcher {
    pub fn new(sender: crate::AppSender) -> Result<Self, String> {
        let path = play_time_path();
        let watch_dir = path.parent().ok_or("no parent for play_time.txt")?;

        let (tx, rx) = std::sync::mpsc::channel();
        let nw = RecommendedWatcher::new(tx, NotifyConfig::default())
            .map_err(|e| e.to_string())?;
        let watcher = std::sync::Arc::new(std::sync::Mutex::new(nw));

        {
            let mut w = watcher.lock().unwrap();
            w.watch(watch_dir, RecursiveMode::NonRecursive)
                .map_err(|e| format!("watch {}: {}", watch_dir.display(), e))?;
        }

        let thread_sender = sender;
        std::thread::spawn(move || {
            let mut last_event = Instant::now();
            let mut pending = false;
            const DEBOUNCE: Duration = Duration::from_secs(2);

            loop {
                match rx.recv_timeout(Duration::from_millis(500)) {
                    Ok(Ok(event)) => {
                        let is_play_time = event.paths.iter().any(|p| {
                            p.file_name().and_then(|n| n.to_str()) == Some("play_time.txt")
                        }) && matches!(
                            event.kind,
                            EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
                        );
                        if is_play_time {
                            last_event = Instant::now();
                            pending = true;
                        }
                    }
                    Ok(Err(e)) => {
                        eprintln!("shadPS4 watcher error: {}", e);
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        if pending && last_event.elapsed() >= DEBOUNCE {
                            pending = false;
                            let _ = thread_sender.send(AppMessage::ShadPS4PlaytimeChanged);
                        }
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
        });

        Ok(ShadPS4Watcher { _watcher: watcher })
    }
}
