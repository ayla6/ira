use rusqlite::Connection;
use std::path::PathBuf;

/// A game as known to Lutris (read from `~/.local/share/lutris/pga.db`).
/// Lutris is the source of truth for the game list; our DB only stores the
/// matching to an achievement source (Steam/GOG) plus user preferences.
#[derive(Clone)]
pub struct LutrisGame {
    /// Lutris internal numeric id — the stable link we store in our DB.
    pub id: i64,
    pub name: String,
    pub slug: String,
    pub runner: String,
    /// "steam", "gog", or "" for games with no store service.
    pub service: String,
    /// Steam app id (service=steam) or GOG product id (service=gog).
    pub service_id: String,
    pub installed: bool,
    /// Playtime in hours.
    pub playtime: f64,
    /// Unix timestamp of last play.
    pub lastplayed: i64,
    pub platform: String,
    /// Install directory (where the game + its save files live).
    pub directory: String,
}

pub fn lutris_db_path() -> PathBuf {
    xdg::BaseDirectories::new()
        .get_data_home()
        .map(|p| p.join("lutris").join("pga.db"))
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("lutris")
                .join("pga.db")
        })
}

/// Read every game from the Lutris database, ordered by name.
pub fn load_lutris_games() -> Result<Vec<LutrisGame>, String> {
    let path = lutris_db_path();
    let conn = Connection::open(&path).map_err(|e| format!("open {}: {}", path.display(), e))?;
    let mut stmt = conn
        .prepare(
            "SELECT id, name, slug, runner, service, service_id, installed, playtime, lastplayed, platform, directory
             FROM games ORDER BY name",
        )
        .map_err(|e| e.to_string())?;
    let games = stmt
        .query_map([], |row| {
            Ok(LutrisGame {
                id: row.get(0)?,
                name: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                slug: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                runner: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                service: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                service_id: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
                installed: row
                    .get::<_, Option<i64>>(6)?
                    .map(|i| i != 0)
                    .unwrap_or(false),
                playtime: row.get::<_, Option<f64>>(7)?.unwrap_or(0.0),
                lastplayed: row.get::<_, Option<i64>>(8)?.unwrap_or(0),
                platform: row.get::<_, Option<String>>(9)?.unwrap_or_default(),
                directory: row.get::<_, Option<String>>(10)?.unwrap_or_default(),
            })
        })
        .map_err(|e| e.to_string())?;
    let mut result = Vec::new();
    for g in games {
        result.push(g.map_err(|e| e.to_string())?);
    }
    Ok(result)
}

/// Read just `(id, playtime, lastplayed)` for every game in pga.db.
pub fn load_lutris_playtime() -> Result<Vec<(i64, f64, i64)>, String> {
    let path = lutris_db_path();
    let conn = Connection::open(&path).map_err(|e| format!("open {}: {}", path.display(), e))?;
    let mut stmt = conn
        .prepare("SELECT id, playtime, lastplayed FROM games")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<f64>>(1)?.unwrap_or(0.0),
                row.get::<_, Option<i64>>(2)?.unwrap_or(0),
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut result = Vec::new();
    for r in rows {
        result.push(r.map_err(|e| e.to_string())?);
    }
    Ok(result)
}
