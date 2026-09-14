//! ScreenScraper.fr metadata search, following the ES-DE client
//! (`references/emulationstation-de/es-app/src/scrapers/ScreenScraper.cpp`).
//! Metadata only — names, dates, credits, genres, players, ratings,
//! synopses, plus the media URLs (PS1 squares and the title-screen
//! fallback use them); everything else
//! keeps coming from Steam, SGDB, and RetroAchievements.

use crate::screenscraper_creds::{ScraperCreds, SOFT_NAME};
use crate::SteamDataClient;
use ira_models::screenscraper_system_id;
use serde::Deserialize;

const API_URL_BASE: &str = "https://www.screenscraper.fr/api2";

/// A game candidate as ScreenScraper answered it: every region's date,
/// every language's synopsis, and id-referenced companies and genres —
/// plus the ES-DE fallback picks for the display fields.
#[derive(Debug, Default, Clone)]
pub struct ScrapedGame {
    pub ss_id: String,
    pub name: String,
    /// The display date: the first dated region in the fallback order.
    pub release_date: String,
    /// Every dated region as `(region, YYYY-MM-DD)` pairs.
    pub release_dates: Vec<(String, String)>,
    pub developers: Vec<ira_models::ScraperEntity>,
    pub publishers: Vec<ira_models::ScraperEntity>,
    /// All English genres, the primary one first.
    pub genres: Vec<ira_models::ScraperEntity>,
    pub players: String,
    /// ScreenScraper's community note, 0..20 (-1 = none given).
    pub rating: f64,
    /// Age-rating board entries (CERO / PEGI / ESRB / ...).
    pub classifications: Vec<ira_models::ScraperClassification>,
    /// Every synopsis as `(language, text)` pairs, entities cleaned.
    pub synopses: Vec<(String, String)>,
    /// Screenshot media URL.
    pub screenshot: Option<String>,
    /// Region-picked 2D box URL — the PS1 square's source.
    pub box2d: Option<String>,
    /// Region-picked title screen URL — the fallback for consoles whose
    /// cover art runs ugly, 3DS above all.
    pub title_screen: Option<String>,
}

impl ScrapedGame {
    /// The storable part: everything but the media URLs and display name.
    pub fn metadata(&self, release_timestamp: i64) -> ira_models::ScraperMetadata {
        ira_models::ScraperMetadata {
            ss_id: self.ss_id.clone(),
            release_date: self.release_date.clone(),
            release_timestamp,
            release_dates: self.release_dates.clone(),
            developers: self.developers.clone(),
            publishers: self.publishers.clone(),
            genres: self.genres.clone(),
            players: self.players.clone(),
            rating: self.rating,
            classifications: self.classifications.clone(),
            synopses: self.synopses.clone(),
        }
    }
}

/// The exact-match URL: `jeuInfos.php` with the ROM name and optional
/// file hash. ScreenScraper treats a hit here as authoritative.
pub fn game_info_url(
    creds: &ScraperCreds,
    rom_nom: &str,
    platform_id: &str,
    md5: Option<(&str, u64)>,
) -> String {
    let mut url = format!(
        "{API_URL_BASE}/jeuInfos.php?{}&softname={}&output=xml&romnom={}",
        creds.auth_params(),
        urlencode(SOFT_NAME),
        urlencode(rom_nom),
    );
    if let Some(system) = screenscraper_system_id(platform_id) {
        url.push_str(&format!("&systemeid={system}"));
    }
    if let Some((md5, size)) = md5 {
        url.push_str(&format!("&md5={}&romtaille={size}", md5.to_lowercase()));
    }
    url
}

/// The by-id URL: re-fetch one known game without searching.
pub fn game_info_by_id_url(creds: &ScraperCreds, ss_id: &str) -> String {
    format!(
        "{API_URL_BASE}/jeuInfos.php?{}&softname={}&output=xml&gameid={}",
        creds.auth_params(),
        urlencode(SOFT_NAME),
        urlencode(ss_id),
    )
}

/// The wide search URL: `jeuRecherche.php` text matching, for the manual
/// match dialog.
pub fn search_url(creds: &ScraperCreds, term: &str, platform_id: &str) -> String {
    let mut url = format!(
        "{API_URL_BASE}/jeuRecherche.php?{}&softname={}&output=xml&recherche={}",
        creds.auth_params(),
        urlencode(SOFT_NAME),
        urlencode(term),
    );
    if let Some(system) = screenscraper_system_id(platform_id) {
        url.push_str(&format!("&systemeid={system}"));
    }
    url
}

/// Regions tried in order for names, dates and media — ES-DE's fallback
/// chain (`{region}, wor, us, ss, eu, jp`), fixed to Ira's region: US
/// first, then world.
fn region_preference() -> Vec<&'static str> {
    vec!["us", "wor", "ss", "eu", "jp"]
}

/// Languages tried in order for synopses and genres.
fn language_preference() -> Vec<&'static str> {
    vec!["en", "wor", "fr", "de", "es", "it", "jp"]
}

/// First `text` among `items` whose attribute matches the preference
/// order — the shared shape of `noms/nom[@region]`, `dates/date[@region]`,
/// `synopsis/synopsis[@langue]` and `genres/genre[@langue]`.
fn pick<'a>(preference: &[&str], items: &'a [(String, String)]) -> Option<&'a str> {
    for wanted in preference {
        for (key, text) in items {
            if key == wanted && !text.is_empty() {
                return Some(text);
            }
        }
    }
    // Nothing preferred: take whatever exists, so regionless or
    // other-language entries still surface.
    items.iter().map(|(_, text)| text.as_str()).find(|t| !t.is_empty())
}

// ——— XML model ———
// The API answers in XML whose shape ES-DE documents by example:
// Data > ssuser + jeux > jeu* with attributed children. Parsed with
// quick-xml's serde support; missing nodes deserialize to None.

#[derive(Debug, Deserialize)]
struct SsData {
    #[serde(default)]
    jeux: Option<SsJeux>,
}

#[derive(Debug, Deserialize)]
struct SsJeux {
    #[serde(default, rename = "jeu")]
    games: Vec<SsJeu>,
}

#[derive(Debug, Deserialize)]
struct SsJeu {
    #[serde(rename = "@id", default)]
    id: String,
    #[serde(default)]
    noms: Option<SsAttributed>,
    #[serde(default)]
    dates: Option<SsAttributed>,
    #[serde(default, rename = "developpeur")]
    developers: Vec<SsEntity>,
    #[serde(default, rename = "editeur")]
    publishers: Vec<SsEntity>,
    #[serde(default)]
    genres: Option<SsGenres>,
    #[serde(default)]
    joueurs: Option<String>,
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    synopsis: Option<SsAttributed>,
    #[serde(default)]
    classifications: Option<SsClassifications>,
    #[serde(default)]
    medias: Option<SsMedias>,
}

/// A referenced entity: `<developpeur id="2911">Chunsoft</developpeur>`.
#[derive(Debug, Deserialize)]
struct SsEntity {
    #[serde(rename = "@id", default)]
    id: String,
    #[serde(default, rename = "$text")]
    name: String,
}

/// Children like `<nom region="us">Text</nom>` — an attribute plus text,
/// repeated.
#[derive(Debug, Deserialize)]
struct SsAttributed {
    #[serde(default, rename = "nom")]
    nom: Vec<SsText>,
    #[serde(default, rename = "date")]
    date: Vec<SsText>,
    #[serde(default, rename = "synopsis")]
    synopsis: Vec<SsText>,
}

#[derive(Debug, Deserialize)]
struct SsText {
    #[serde(rename = "@region", default)]
    region: String,
    #[serde(rename = "@langue", default)]
    langue: String,
    #[serde(default, rename = "$text")]
    text: String,
}

#[derive(Debug, Deserialize)]
struct SsGenres {
    #[serde(default, rename = "genre")]
    genre: Vec<SsGenre>,
}

#[derive(Debug, Deserialize)]
struct SsGenre {
    #[serde(rename = "@id", default)]
    id: String,
    #[serde(rename = "@primary", default)]
    primary: String,
    #[serde(rename = "@langue", default)]
    langue: String,
    #[serde(default, rename = "$text")]
    text: String,
}

#[derive(Debug, Deserialize)]
struct SsClassifications {
    #[serde(default, rename = "classification")]
    classification: Vec<SsClassificationRow>,
}

#[derive(Debug, Deserialize)]
struct SsClassificationRow {
    #[serde(rename = "@type", default)]
    kind: String,
    #[serde(rename = "@id", default)]
    id: String,
    #[serde(default, rename = "$text")]
    name: String,
}

#[derive(Debug, Deserialize)]
struct SsMedias {
    #[serde(default, rename = "media")]
    media: Vec<SsMedia>,
}

#[derive(Debug, Deserialize)]
struct SsMedia {
    #[serde(rename = "@type", default)]
    kind: String,
    #[serde(rename = "@region", default)]
    region: String,
    #[serde(default, rename = "$text")]
    url: String,
}

/// Parse a `jeuRecherche`/`jeuInfos` XML answer into candidates. Applies
/// ES-DE's cleanups: HTML entities ScreenScraper leaves in the text, the
/// "ZZZ(notgame)" placeholder results, and duplicate game ids that one
/// multi-system query produces.
pub fn parse_games(xml: &str) -> Result<Vec<ScrapedGame>, String> {
    // Plain French text answers, not XML. A rom miss is an empty result;
    // anything else — rejected credentials above all — is a real error
    // the caller must see instead of "no match".
    let trimmed = xml.trim_start();
    if trimmed.starts_with("Erreur : Rom") {
        return Ok(Vec::new());
    }
    if trimmed.starts_with("Erreur") {
        if trimmed.contains("login") || trimmed.contains("identifiants") {
            return Err(
                "ScreenScraper rejected the developer credentials — the per-application id and password from screenscraper.fr, not your account login"
                    .to_string(),
            );
        }
        let excerpt: String = trimmed.chars().take(120).collect();
        return Err(format!("ScreenScraper error: {excerpt}"));
    }
    let data: SsData = quick_xml::de::from_str(xml)
        .map_err(|e| format!("ScreenScraper returned unreadable XML: {e}"))?;
    let Some(jeux) = data.jeux else {
        return Ok(Vec::new());
    };
    let mut games: Vec<ScrapedGame> = jeux
        .games
        .iter()
        .map(scraped_game)
        .filter(|g| {
            !g.name
                .to_uppercase()
                .starts_with("ZZZ(NOTGAME)")
        })
        .collect();
    let mut seen = std::collections::HashSet::new();
    games.retain(|g| seen.insert(g.ss_id.clone()));
    Ok(games)
}

fn scraped_game(jeu: &SsJeu) -> ScrapedGame {
    let regions = region_preference();
    let languages = language_preference();
    let name = jeu
        .noms
        .as_ref()
        .and_then(|n| {
            let items: Vec<_> = n.nom.iter().map(|n| (n.region.clone(), n.text.clone())).collect();
            pick(&regions, &items).map(str::to_string)
        })
        .unwrap_or_default()
        .replace("&nbsp;", " ")
        .replace("&#x26;", "&")
        .replace("&#39;", "\u{2019}")
        .replace('\n', "");
    let release_dates: Vec<(String, String)> = jeu
        .dates
        .as_ref()
        .map(|d| {
            d.date
                .iter()
                .map(|d| (d.region.clone(), d.text.clone()))
                .collect()
        })
        .unwrap_or_default();
    let release_date = pick(&regions, &release_dates)
        .unwrap_or_default()
        .to_string();
    // English genres, primary first.
    let genres: Vec<ira_models::ScraperEntity> = jeu
        .genres
        .as_ref()
        .map(|g| {
            let mut rows: Vec<&SsGenre> = g
                .genre
                .iter()
                .filter(|genre| genre.langue == "en")
                .collect();
            rows.sort_by_key(|genre| if genre.primary == "1" { 0 } else { 1 });
            rows.iter()
                .map(|genre| ira_models::ScraperEntity {
                    id: genre.id.clone(),
                    name: genre.text.clone(),
                })
                .collect()
        })
        .unwrap_or_default();
    let rating = jeu
        .note
        .as_deref()
        .and_then(|n| n.trim().parse::<f64>().ok())
        .map(|n| n.clamp(0.0, 20.0))
        .unwrap_or(-1.0);
    let classifications = jeu
        .classifications
        .as_ref()
        .map(|c| {
            c.classification
                .iter()
                .map(|row| ira_models::ScraperClassification {
                    kind: row.kind.clone(),
                    id: row.id.clone(),
                    name: row.name.clone(),
                })
                .collect()
        })
        .unwrap_or_default();
    let synopses: Vec<(String, String)> = jeu
        .synopsis
        .as_ref()
        .map(|s| {
            s.synopsis
                .iter()
                .map(|synopsis| {
                    let text = collapse_spaces(
                        &synopsis
                            .text
                            .replace("&nbsp;", " ")
                            .replace("&quot;", "\"")
                            .replace("&copy;", "\u{a9}")
                            .replace("&#039;", "'")
                            .replace("&#39;", "'"),
                    );
                    (synopsis.langue.clone(), text)
                })
                .collect()
        })
        .unwrap_or_default();
    // Stored preferred-language first, so a display layer can take the
    // first entry as the synopsis and still find the rest by language.
    let mut synopses = synopses;
    synopses.sort_by_key(|(langue, _)| {
        languages
            .iter()
            .position(|wanted| wanted == langue)
            .unwrap_or(languages.len())
    });
    let medias = jeu.medias.as_ref();
    let screenshot = medias.and_then(|m| media_url(&m.media, "ss", &regions));
    let box2d = medias.and_then(|m| media_url(&m.media, "box-2D", &regions));
    let title_screen = medias.and_then(|m| media_url(&m.media, "sstitle", &regions));
    ScrapedGame {
        ss_id: jeu.id.clone(),
        name,
        release_date,
        release_dates,
        developers: entity_list(&jeu.developers),
        publishers: entity_list(&jeu.publishers),
        genres,
        players: jeu.joueurs.as_deref().unwrap_or_default().trim().to_string(),
        rating,
        classifications,
        synopses,
        screenshot,
        box2d,
        title_screen,
    }
}

/// The referenced entities, names trimmed — ScreenScraper repeats
/// `<developpeur>`/`<editeur>` elements when several apply.
fn entity_list(entities: &[SsEntity]) -> Vec<ira_models::ScraperEntity> {
    entities
        .iter()
        .map(|entity| ira_models::ScraperEntity {
            id: entity.id.clone(),
            name: entity.name.trim().to_string(),
        })
        .collect()
}

/// The region-picked URL for one media type; the URL's spaces are
/// escaped because ScreenScraper echoes `softname` into media paths.
fn media_url(medias: &[SsMedia], kind: &str, regions: &[&str]) -> Option<String> {
    let matches: Vec<(String, String)> = medias
        .iter()
        .filter(|m| m.kind == kind)
        .map(|m| (m.region.clone(), m.url.clone()))
        .collect();
    pick(regions, &matches).map(|url| url.replace(' ', "%20"))
}

impl SteamDataClient {
    /// Run a ScreenScraper wide search. Empty when credentials are not
    /// configured; errors surface for the dialog to show.
    pub fn screenscraper_search(
        &self,
        creds: &ScraperCreds,
        term: &str,
        platform_id: &str,
    ) -> Result<Vec<ScrapedGame>, String> {
        if !creds.is_configured() {
            return Err("ScreenScraper credentials not configured".to_string());
        }
        self.screenscraper_get(&search_url(creds, term, platform_id))
    }

    /// Run a ScreenScraper exact ROM lookup (name + optional hash).
    pub fn screenscraper_rom_lookup(
        &self,
        creds: &ScraperCreds,
        rom_nom: &str,
        platform_id: &str,
        md5: Option<(&str, u64)>,
    ) -> Result<Vec<ScrapedGame>, String> {
        if !creds.is_configured() {
            return Err("ScreenScraper credentials not configured".to_string());
        }
        self.screenscraper_get(&game_info_url(creds, rom_nom, platform_id, md5))
    }

    /// Re-fetch one known game by its ScreenScraper id.
    pub fn screenscraper_game(
        &self,
        creds: &ScraperCreds,
        ss_id: &str,
    ) -> Result<Vec<ScrapedGame>, String> {
        if !creds.is_configured() {
            return Err("ScreenScraper credentials not configured".to_string());
        }
        self.screenscraper_get(&game_info_by_id_url(creds, ss_id))
    }

    /// Download one ScreenScraper media URL (screenshot, box) — the media
    /// host is plain HTTPS, no credentials.
    pub fn screenscraper_media(&self, url: &str) -> Result<Vec<u8>, String> {
        let resp = self
            .http
            .get(url)
            .send()
            .map_err(|e| format!("ScreenScraper media download failed: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("ScreenScraper media download failed: {}", resp.status()));
        }
        resp.bytes()
            .map(|b| b.to_vec())
            .map_err(|e| format!("ScreenScraper media download failed: {e}"))
    }

    fn screenscraper_get(&self, url: &str) -> Result<Vec<ScrapedGame>, String> {
        let _s = tracing::info_span!("screenscraper_get", url).entered();
        let resp = self
            .http
            .get(url)
            .send()
            .map_err(|e| format!("ScreenScraper request failed: {e}"))?;
        let status = resp.status();
        let body = resp
            .text()
            .map_err(|e| format!("ScreenScraper request failed: {e}"))?;
        if !status.is_success() {
            return Err(status_hint(status.as_u16(), &body));
        }
        parse_games(&body)
    }
}

/// The documented failure meanings, so a 429 does not read as a mystery.
fn status_hint(status: u16, body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.starts_with("Erreur") {
        return format!("ScreenScraper error: {}", trimmed);
    }
    let hint = match status {
        401 => " — the API is closed to non-members right now (server saturation)",
        403 => " — the developer credentials were rejected",
        423 => " — the API is fully closed (server trouble)",
        426 => " — ScreenScraper blacklisted this software version",
        429 => " — too many requests: the batch will need to slow down",
        430 => " — the daily scrape quota is used up, try again tomorrow",
        431 => " — too many unrecognized files today, try again tomorrow",
        _ => "",
    };
    format!("ScreenScraper request failed: {status}{hint}")
}

/// Collapse the double spaces the entity replacements leave behind.
fn collapse_spaces(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut previous_space = false;
    for c in text.chars() {
        let space = c == ' ';
        if !(space && previous_space) {
            out.push(c);
        }
        previous_space = space;
    }
    out
}

fn urlencode(s: &str) -> String {
    crate::util::urlencode(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_game_info_url_carries_credentials_system_and_hash() {
        let creds = ScraperCreds {
            dev_id: "ira".into(),
            dev_password: "devkey".into(),
            user: "aya 7".into(),
            password: "s3cret&".into(),
        };
        let url = game_info_url(&creds, "Final Fantasy VII (USA)", "psx", Some(("ABCDEF", 711_000)));
        assert!(url.starts_with("https://www.screenscraper.fr/api2/jeuInfos.php?"));
        assert!(url.contains("devid=ira"));
        assert!(url.contains("devpassword=devkey"));
        // The account login rides along as quota attribution.
        assert!(url.contains("ssid=aya%207"));
        assert!(url.contains("sspassword=s3cret%26"));
        assert!(url.contains("systemeid=57"));
        assert!(url.contains("md5=abcdef"));
        assert!(url.contains("romtaille=711000"));
        // Without an account set, no ssid pair is sent.
        let anon = ScraperCreds {
            dev_id: "ira".into(),
            dev_password: "devkey".into(),
            ..Default::default()
        };
        let url = game_info_url(&anon, "Doom", "wine", None);
        assert!(!url.contains("ssid"));
        assert!(!url.contains("systemeid"));
    }

    #[test]
    fn test_search_url_targets_jeurecherche() {
        let creds = ScraperCreds {
            dev_id: "ira".into(),
            dev_password: "pw".into(),
            ..Default::default()
        };
        let url = search_url(&creds, "zelda", "snes");
        assert!(url.contains("jeuRecherche.php"));
        assert!(url.contains("recherche=zelda"));
        assert!(url.contains("systemeid=4"));
    }

    const SEARCH_XML: &str = r#"<Data>
      <ssuser><id>1</id><niveau>3</niveau><requeststoday>10</requeststoday><maxrequestsperday>15000</maxrequestsperday></ssuser>
      <jeux>
        <jeu id="2124">
          <noms>
            <nom region="jp">ドラゴンクエストI・II</nom>
            <nom region="us">Dragon Quest I &amp; II</nom>
            <nom region="wor">Dragon Quest I &amp; II (Wor)</nom>
          </noms>
          <dates>
            <date region="jp">1993-12-18</date>
            <date region="us">1993-12-18</date>
          </dates>
          <developpeur id="2911"> Chunsoft </developpeur>
          <developpeur id="2912">Nintendo</developpeur>
          <editeur id="1299">Enix</editeur>
          <genres>
            <genre id="2620" primary="1" langue="en">Role Playing Game</genre>
            <genre id="2406" langue="en">Adventure</genre>
            <genre id="2621" langue="fr">Jeu de r&#244;le</genre>
          </genres>
          <joueurs>1-4</joueurs>
          <note>18</note>
          <classifications>
            <classification type="CERO" id="277">CERO:A</classification>
            <classification type="PEGI" id="279">PEGI:3</classification>
            <classification type="ESRB" id="285">ESRB:E10+</classification>
          </classifications>
          <synopsis>
            <synopsis langue="fr">Le royaume...</synopsis>
            <synopsis langue="en">The first two Dragon Quest &quot;quests&quot;, &amp;nbsp;together.</synopsis>
          </synopsis>
          <medias>
            <media type="ss" region="us" format="png">https://ss.example/dq2_us.png</media>
            <media type="ss" region="wor" format="png">https://ss.example/dq2_wor.png</media>
            <media type="box-2D" region="us" format="png">https://ss.example/dq2_box.png</media>
            <media type="sstitle" region="us" format="png">https://ss.example/dq2_title.png</media>
          </medias>
        </jeu>
        <jeu id="2124">
          <noms><nom region="us">Dragon Quest I &amp; II (dup)</nom></noms>
        </jeu>
        <jeu id="999">
          <noms><nom region="us">ZZZ(notgame):Fichier Annexes - Non Jeux</nom></noms>
        </jeu>
      </jeux>
    </Data>"#;

    #[test]
    fn test_parse_games_applies_fallbacks_and_cleanup() {
        let games = parse_games(SEARCH_XML).unwrap();
        // The duplicate id and the ZZZ(notgame) placeholder are gone.
        assert_eq!(games.len(), 1);
        let game = &games[0];
        assert_eq!(game.ss_id, "2124");
        // Region preference keeps the US name over the world one, with
        // the numeric entity decoded.
        assert_eq!(game.name, "Dragon Quest I & II");
        assert_eq!(game.release_date, "1993-12-18");
        // Every region's date lands in the list.
        assert_eq!(
            game.release_dates,
            vec![
                ("jp".to_string(), "1993-12-18".to_string()),
                ("us".to_string(), "1993-12-18".to_string())
            ]
        );
        // Companies come with their ScreenScraper ids; a second
        // <developpeur> element lands as another entry.
        assert_eq!(
            game.developers
                .iter()
                .map(|d| (d.id.as_str(), d.name.as_str()))
                .collect::<Vec<_>>(),
            vec![("2911", "Chunsoft"), ("2912", "Nintendo")]
        );
        assert_eq!(
            game.publishers
                .iter()
                .map(|p| (p.id.as_str(), p.name.as_str()))
                .collect::<Vec<_>>(),
            vec![("1299", "Enix")]
        );
        // English genres, primary first, ids kept.
        assert_eq!(
            game.genres
                .iter()
                .map(|g| (g.id.as_str(), g.name.as_str()))
                .collect::<Vec<_>>(),
            vec![("2620", "Role Playing Game"), ("2406", "Adventure")]
        );
        assert_eq!(game.players, "1-4");
        assert_eq!(game.rating, 18.0);
        // All three boards arrive with their ids.
        assert_eq!(game.classifications.len(), 3);
        assert_eq!(game.classifications[1].kind, "PEGI");
        assert_eq!(game.classifications[1].id, "279");
        assert_eq!(game.classifications[1].name, "PEGI:3");
        // Every language's synopsis is kept, entities cleaned up.
        assert_eq!(game.synopses.len(), 2);
        assert_eq!(
            game.synopses.iter().find(|(l, _)| l == "en").map(|(_, t)| t.as_str()),
            Some("The first two Dragon Quest \"quests\", together.")
        );
        // Stored preferred-language first: English sorts ahead of French.
        assert_eq!(
            game.synopses.first().map(|(l, _)| l.as_str()),
            Some("en")
        );
        // Region preference for media: us is preferred over wor.
        assert_eq!(game.screenshot.as_deref(), Some("https://ss.example/dq2_us.png"));
        assert_eq!(game.box2d.as_deref(), Some("https://ss.example/dq2_box.png"));
        assert_eq!(
            game.title_screen.as_deref(),
            Some("https://ss.example/dq2_title.png")
        );
    }

    #[test]
    fn test_parse_games_handles_french_error_text() {
        assert!(parse_games("Erreur : Rom not found").unwrap().is_empty());
    }

    #[test]
    fn test_parse_games_reports_rejected_credentials() {
        let err = parse_games(
            "Erreur de login : Vérifier vos identifiants développeur !",
        )
        .unwrap_err();
        assert!(err.contains("rejected the developer credentials"), "{err}");
        // Other API errors surface their text instead of parsing garbage.
        let err = parse_games("Erreur : le service est indisponible").unwrap_err();
        assert!(err.contains("indisponible"), "{err}");
    }

    #[test]
    fn test_parse_games_rejects_garbage() {
        assert!(parse_games("<not closed").is_err());
    }
}
