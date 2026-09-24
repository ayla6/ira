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
use std::collections::HashSet;
use unicode_normalization::UnicodeNormalization;

const API_URL_BASE: &str = "https://www.screenscraper.fr/api2";

/// A game candidate as ScreenScraper answered it: every region's date,
/// every language's synopsis, and id-referenced companies and genres —
/// plus the ES-DE fallback picks for the display fields.
#[derive(Debug, Default, Clone)]
pub struct ScrapedGame {
    pub ss_id: String,
    pub name: String,
    /// Every region's name for the game as `(region, name)` pairs. Matching
    /// judges all of them — a Japan-only dump carries the Japanese title
    /// while the picked display name is the English one — and the region
    /// tells a tie-break which candidate the dump actually is.
    pub names: Vec<(String, String)>,
    /// The display date: the first dated region in the fallback order.
    pub release_date: String,
    /// Every dated region as `(region, YYYY-MM-DD)` pairs.
    pub release_dates: Vec<(String, String)>,
    pub developers: Vec<ira_models::ScraperEntity>,
    pub publishers: Vec<ira_models::ScraperEntity>,
    /// All English genres, the primary one first.
    pub genres: Vec<ira_models::ScraperEntity>,
    /// The series groupings the source files the entry under.
    pub families: Vec<ira_models::ScraperEntity>,
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
    /// Per-disc physical-media art (`support-2D`), as `(disc number, URL)`
    /// sorted by number. Multi-disc games carry one entry per CD; the
    /// disc-picker tiles use them, with a numbered fallback when absent.
    pub disc_images: Vec<(i32, String)>,
    /// The console the entry belongs to, as ScreenScraper names it
    /// ("Nintendo 64"). Empty when the answer carries no system.
    pub system_name: String,
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
            families: self.families.clone(),
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

/// The whole genre table: the service has no per-name genre search, so
/// the fetched copy *is* the search index.
pub fn genres_list_url(creds: &ScraperCreds) -> String {
    format!(
        "{API_URL_BASE}/genresListe.php?{}&softname={}&output=xml",
        creds.auth_params(),
        urlencode(SOFT_NAME)
    )
}

/// The account's usage counters, from ssuserInfos.php — ScreenScraper
/// requires clients to read and manage these.
#[derive(Debug, Default, Clone)]
pub struct SsUserInfos {
    pub requests_today: i64,
    pub max_requests_per_day: i64,
    pub requests_ko_today: i64,
    pub max_requests_ko_per_day: i64,
    pub max_requests_per_min: i64,
}

impl SsUserInfos {
    /// The daily scrape quota is spent: keep the pass off the API.
    pub fn exhausted(&self) -> bool {
        self.max_requests_per_day > 0 && self.requests_today >= self.max_requests_per_day
    }
}

#[derive(Debug, Deserialize)]
struct SsUserInfosRoot {
    #[serde(default)]
    ssuser: Option<SsUserRaw>,
}

#[derive(Debug, Deserialize)]
struct SsUserRaw {
    #[serde(default, rename = "requeststoday")]
    requests_today: Option<i64>,
    #[serde(default, rename = "maxrequestsperday")]
    max_requests_per_day: Option<i64>,
    #[serde(default, rename = "requestskotoday")]
    requests_ko_today: Option<i64>,
    #[serde(default, rename = "maxrequestskoperday")]
    max_requests_ko_per_day: Option<i64>,
    #[serde(default, rename = "maxrequestspermin")]
    max_requests_per_min: Option<i64>,
}

pub fn user_infos_url(creds: &ScraperCreds) -> String {
    format!(
        "{API_URL_BASE}/ssuserInfos.php?{}&softname={}&output=xml",
        creds.auth_params(),
        urlencode(SOFT_NAME)
    )
}

pub fn parse_user_infos(xml: &str) -> Result<SsUserInfos, String> {
    let root: SsUserInfosRoot = quick_xml::de::from_str(xml)
        .map_err(|e| format!("ScreenScraper returned unreadable XML: {e}"))?;
    let user = root
        .ssuser
        .ok_or_else(|| "ScreenScraper answer carries no user info".to_string())?;
    Ok(SsUserInfos {
        requests_today: user.requests_today.unwrap_or(0),
        max_requests_per_day: user.max_requests_per_day.unwrap_or(0),
        requests_ko_today: user.requests_ko_today.unwrap_or(0),
        max_requests_ko_per_day: user.max_requests_ko_per_day.unwrap_or(0),
        max_requests_per_min: user.max_requests_per_min.unwrap_or(0),
    })
}

/// The exact-match URL by disc serial: disc dumps — `SLES-52005`, chd,
/// rvz, whatever repack — identify themselves through `serialnum` instead
/// of any file digest.
pub fn serial_lookup_url(creds: &ScraperCreds, serial: &str, platform_id: &str) -> String {
    let mut url = format!(
        "{API_URL_BASE}/jeuInfos.php?{}&softname={}&output=xml&serialnum={}",
        creds.auth_params(),
        urlencode(SOFT_NAME),
        urlencode(serial)
    );
    if let Some(system) = screenscraper_system_id(platform_id) {
        url.push_str(&format!("&systemeid={system}"));
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
/// match dialog. An explicit `system` narrows the answer to one
/// ScreenScraper system; `None` searches every system at once — the
/// cross-platform lookup PC games fall back to.
pub fn search_url_scoped(
    creds: &ScraperCreds,
    term: &str,
    system: Option<u32>,
) -> String {
    let mut url = format!(
        "{API_URL_BASE}/jeuRecherche.php?{}&softname={}&output=xml&recherche={}",
        creds.auth_params(),
        urlencode(SOFT_NAME),
        urlencode(term),
    );
    if let Some(system) = system {
        url.push_str(&format!("&systemeid={system}"));
    }
    url
}

/// The wide search URL: `jeuRecherche.php` text matching, narrowed to the
/// ScreenScraper system an Ira platform maps to.
pub fn search_url(creds: &ScraperCreds, term: &str, platform_id: &str) -> String {
    search_url_scoped(creds, term, screenscraper_system_id(platform_id))
}

/// Order search answers so the title closest to `term` comes first: the
/// picker reads top-down, and the source's own answer order is no help
/// to a human picking a match. The score is a Dice coefficient over the
/// character bigrams of the normalized titles, taken over every name the
/// candidate carries — display name and all its region names. Equal
/// scores keep the answer's original order.
pub fn sort_by_similarity(games: &mut Vec<ScrapedGame>, term: &str) {
    let target = title_bigrams(&similarity_fold(term));
    let mut keyed: Vec<(f64, ScrapedGame)> = std::mem::take(games)
        .into_iter()
        .map(|game| {
            let score = std::iter::once(game.name.as_str())
                .chain(game.names.iter().map(|(_, name)| name.as_str()))
                .map(|name| dice(&target, &title_bigrams(&similarity_fold(name))))
                .fold(0.0_f64, f64::max);
            (score, game)
        })
        .collect();
    keyed.sort_by(|a, b| b.0.total_cmp(&a.0));
    *games = keyed.into_iter().map(|(_, game)| game).collect();
}

/// Lowercase, accents folded away (NFD plus the combining marks
/// stripped), punctuation dropped — the source writes "Pokémon" and
/// "Ace Combat 04 : Shattered Skies" where the searcher types neither.
fn similarity_fold(title: &str) -> String {
    title
        .to_lowercase()
        .nfd()
        .filter(|c| !matches!(u32::from(*c), 0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x20D0..=0x20FF))
        .filter(|c| c.is_ascii_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn title_bigrams(title: &str) -> HashSet<[char; 2]> {
    let chars: Vec<char> = title.chars().collect();
    chars.windows(2).map(|w| [w[0], w[1]]).collect()
}

/// Dice over two bigram sets: twice the shared over the total. An empty
/// side (a one-character title) scores 0 — a term that short leaves the
/// answer in its original order.
fn dice(a: &HashSet<[char; 2]>, b: &HashSet<[char; 2]>) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    2.0 * a.intersection(b).count() as f64 / (a.len() + b.len()) as f64
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
    /// `jeuInfos` answers with the one game directly under `<Data>`, no
    /// `<jeux>` wrapper.
    #[serde(default)]
    jeu: Option<SsJeu>,
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
    /// The series the entry belongs to:
    /// `<familles><famille id="732" nom="Dragon Quest"/></familles>`.
    #[serde(default, rename = "familles")]
    families: Option<SsFamilles>,
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
    /// The console the entry belongs to:
    /// `<systeme id="138">PC Windows</systeme>`.
    #[serde(default, rename = "systeme")]
    system: Option<SsEntity>,
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

/// `<familles>` — the series groupings the community files entries
/// under ("Persona", "Megami Tensei").
#[derive(Debug, Deserialize)]
struct SsFamilles {
    #[serde(default, rename = "famille")]
    famille: Vec<SsFamille>,
}

/// One family: the name rides the `nom` attribute; some answers carry
/// it as text content instead, so both are read and `nom` wins.
#[derive(Debug, Deserialize)]
struct SsFamille {
    #[serde(rename = "@id", default)]
    id: String,
    #[serde(rename = "@nom", default)]
    nom: String,
    #[serde(default, rename = "$text")]
    text: String,
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
    #[serde(default, rename = "$text")]
    value: String,
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
    /// The support (disc/cartridge) number this media shows, 1-based.
    /// Present on the per-disc media of multi-disc games; absent means 1.
    #[serde(rename = "@support", default)]
    support: String,
    #[serde(default, rename = "$text")]
    url: String,
}

/// The miss text the exact endpoints answer with — "Erreur : Rom/Iso/
/// Dossier non trouvée !" — whatever HTTP status carries it.
fn is_rom_miss(body: &str) -> bool {
    body.trim_start().starts_with("Erreur : Rom")
}

/// Parse a `jeuRecherche`/`jeuInfos` XML answer into candidates. Applies
/// ES-DE's cleanups: HTML entities ScreenScraper leaves in the text, the
/// "ZZZ(notgame)" placeholder results, and duplicate game ids that one
/// multi-system query produces.
/// The game's page on the ScreenScraper site — what "open in browser"
/// affordances point at.
pub fn game_page_url(ss_id: &str) -> String {
    format!("https://www.screenscraper.fr/gameinfos.php?gameid={ss_id}")
}

pub fn parse_games(xml: &str) -> Result<Vec<ScrapedGame>, String> {
    // Plain French text answers, not XML. A rom miss is an empty result;
    // anything else — rejected credentials above all — is a real error
    // the caller must see instead of "no match".
    if is_rom_miss(xml) {
        return Ok(Vec::new());
    }
    let trimmed = xml.trim_start();
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
    // jeuRecherche wraps its table in <jeux>; jeuInfos (md5, serial, gameid)
    // puts the single game straight under <Data>.
    let source_games = match data.jeux {
        Some(jeux) => jeux.games,
        None => data.jeu.into_iter().collect::<Vec<_>>(),
    };
    let mut games: Vec<ScrapedGame> = source_games
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
    scraped_game_with(jeu, &region_preference())
}

fn scraped_game_with(jeu: &SsJeu, regions: &[&str]) -> ScrapedGame {
    let languages = language_preference();
    let name = jeu
        .noms
        .as_ref()
        .and_then(|n| {
            let items: Vec<_> = n.nom.iter().map(|n| (n.region.clone(), n.text.clone())).collect();
            pick(regions, &items).map(str::to_string)
        })
        .unwrap_or_default()
        .replace("&nbsp;", " ")
        .replace("&#x26;", "&")
        .replace("&#39;", "\u{2019}")
        .replace('\n', "")
        // ScreenScraper's French typographic spaces leak into non-French
        // names: "007 : Everything or Nothing" reads as "007: Everything
        // or Nothing" everywhere else.
        .replace(" : ", ": ");
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
    let release_date = pick(regions, &release_dates)
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
    let families: Vec<ira_models::ScraperEntity> = jeu
        .families
        .as_ref()
        .map(|f| {
            f.famille
                .iter()
                .filter(|famille| !famille.id.is_empty())
                .map(|famille| ira_models::ScraperEntity {
                    id: famille.id.clone(),
                    name: if famille.nom.is_empty() {
                        famille.text.clone()
                    } else {
                        famille.nom.clone()
                    },
                })
                .filter(|famille| !famille.name.is_empty())
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
                    value: row.value.trim().to_string(),
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
    let screenshot = medias.and_then(|m| media_url(&m.media, "ss", regions));
    let box2d = medias.and_then(|m| media_url(&m.media, "box-2D", regions));
    let title_screen = medias.and_then(|m| media_url(&m.media, "sstitle", regions));
    let disc_images = medias
        .map(|m| disc_media_urls(&m.media, regions, false))
        .unwrap_or_default();
    // Every region's name, entities decoded so comparisons see real
    // characters, deduplicated.
    let mut names: Vec<(String, String)> = Vec::new();
    if let Some(noms) = jeu.noms.as_ref() {
        for nom in &noms.nom {
            let text = decode_entities(&nom.text).trim().to_string();
            if !text.is_empty() && !names.iter().any(|(_, n)| *n == text) {
                names.push((nom.region.clone(), text));
            }
        }
    }
    ScrapedGame {
        ss_id: jeu.id.clone(),
        name,
        names,
        release_date,
        release_dates,
        developers: entity_list(&jeu.developers),
        publishers: entity_list(&jeu.publishers),
        genres,
        families,
        players: jeu.joueurs.as_deref().unwrap_or_default().trim().to_string(),
        rating,
        classifications,
        synopses,
        screenshot,
        box2d,
        title_screen,
        disc_images,
        system_name: jeu
            .system
            .as_ref()
            .map(|s| s.name.trim().to_string())
            .unwrap_or_default(),
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

/// ScreenScraper leaves non-standard XML entities in its text; decode the
/// ones that appear in names so comparisons see real characters.
fn decode_entities(text: &str) -> String {
    text.replace("&nbsp;", " ")
        .replace("&#x26;", "&")
        .replace("&quot;", "\"")
        .replace("&#039;", "'")
        .replace("&#39;", "'")
        .replace('\n', "")
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

/// One per-disc art entry: the support number and its region-picked
/// `support-2D` URL, ready for a picker tile.
pub struct DiscMedia {
    pub disc: i32,
    pub png: Vec<u8>,
}

    /// The region-picked `support-2D` URL for every disc the answer carries.
    /// Multi-disc games tag each media node with a `support` attribute
    /// (`<media type="support-2D" region="eu" support="2">`); nodes without
    /// one are disc 1. URLs get the same space-escaping as `media_url`.
    /// A multi-region list is a preference order (the ROM's own region
    /// first); a single region pins exactly that region or nothing.
    pub(crate) fn parse_disc_images(
        xml: &str,
        regions: &[String],
    ) -> Result<Vec<(i32, String)>, String> {
        let data: SsData = quick_xml::de::from_str(xml)
            .map_err(|e| format!("ScreenScraper returned unreadable XML: {e}"))?;
        let jeux: Vec<SsJeu> = match data.jeux {
            Some(jeux) => jeux.games,
            None => data.jeu.into_iter().collect(),
        };
        let Some(jeu) = jeux.into_iter().next() else {
            return Ok(Vec::new());
        };
        let fallback = region_preference();
        let strict = regions.len() == 1;
        let regions: Vec<&str> = if regions.is_empty() {
            fallback
        } else {
            regions.iter().map(String::as_str).collect()
        };
        let Some(medias) = jeu.medias.as_ref() else {
            return Ok(Vec::new());
        };
        Ok(disc_media_urls(&medias.media, &regions, strict))
}

/// The region-picked `support-2D` URL for every disc the answer carries.
/// Multi-disc games tag each media node with a `support` attribute
/// (`<media type="support-2D" region="eu" support="2">`); nodes without
/// one are disc 1. URLs get the same space-escaping as `media_url`.
/// Strict mode skips the usual preferred-region fallback: an explicit
/// region choice returns exactly that region or nothing.
fn disc_media_urls(medias: &[SsMedia], regions: &[&str], strict: bool) -> Vec<(i32, String)> {
    let mut per_disc: std::collections::HashMap<i32, Vec<(String, String)>> = Default::default();
    for media in medias.iter().filter(|m| m.kind == "support-2D") {
        let disc = media.support.parse::<i32>().unwrap_or(1);
        per_disc
            .entry(disc)
            .or_default()
            .push((media.region.clone(), media.url.clone()));
    }
    let mut out: Vec<(i32, String)> = per_disc
        .into_iter()
        .filter_map(|(disc, urls)| {
                let url = if strict {
                    urls.iter().find_map(|(region, url)| {
                        (regions.contains(&region.as_str()) && !url.is_empty())
                            .then_some(url.as_str())
                    })
                } else {
                    pick(regions, &urls)
                };
            url.map(|url| (disc, url.replace(' ', "%20")))
        })
        .collect();
    out.sort_by_key(|(disc, _)| *disc);
    out
}

// ——— genre table ———
// `genresListe.php` answers `<genre><id>..</id><nom_en>..</nom_en>..` rows
// covering every genre the service knows, so one cached request serves
// every picker forever.

/// One genre from the whole-table listing: stable id, display name,
/// parent id (`0` = top level).
#[derive(Debug, Clone)]
pub struct GenreListEntry {
    pub id: String,
    pub name: String,
    pub parent: String,
}

#[derive(Debug, Deserialize)]
struct SsGenresRoot {
    #[serde(default)]
    genres: Option<SsGenreTable>,
}

#[derive(Debug, Deserialize)]
struct SsGenreTable {
    #[serde(default, rename = "genre")]
    genres: Vec<SsGenreListRow>,
}

#[derive(Debug, Deserialize)]
struct SsGenreListRow {
    #[serde(default)]
    id: String,
    #[serde(default, rename = "nom_en")]
    name_en: String,
    #[serde(default, rename = "nom_fr")]
    name_fr: String,
    #[serde(default)]
    parent: String,
}

/// Parse the genre table; the English name is preferred with the French
/// one as fallback, and nameless or idless rows drop out.
pub fn parse_genres_list(xml: &str) -> Result<Vec<GenreListEntry>, String> {
    let root: SsGenresRoot = quick_xml::de::from_str(xml)
        .map_err(|e| format!("ScreenScraper returned unreadable XML: {e}"))?;
    Ok(root
        .genres
        .map(|table| table.genres)
        .unwrap_or_default()
        .into_iter()
        .filter(|row| !row.id.is_empty())
        .map(|row| GenreListEntry {
            name: if row.name_en.is_empty() {
                row.name_fr
            } else {
                row.name_en
            },
            id: row.id,
            parent: row.parent,
        })
        .filter(|row| !row.name.is_empty())
        .collect())
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
        let system = screenscraper_system_id(platform_id);
        self.screenscraper_search_in(creds, term, system)
    }

    /// Run a ScreenScraper wide search narrowed to one system, or across
    /// every system when `system` is None — the PC games' cross-platform
    /// lookup.
    pub fn screenscraper_search_in(
        &self,
        creds: &ScraperCreds,
        term: &str,
        system: Option<u32>,
    ) -> Result<Vec<ScrapedGame>, String> {
        if !creds.is_configured() {
            return Err("ScreenScraper credentials not configured".to_string());
        }
        self.screenscraper_get(&search_url_scoped(creds, term, system))
    }

    /// Fetch the whole genre table. The service has no per-name genre
    /// search, so the fetched copy is the search index.
    pub fn genres_list(
        &self,
        creds: &ScraperCreds,
    ) -> Result<Vec<GenreListEntry>, String> {
        if !creds.is_configured() {
            return Err("ScreenScraper credentials not configured".to_string());
        }
        let xml = self
            .http
            .get(genres_list_url(creds))
            .send()
            .map_err(|e| format!("ScreenScraper request failed: {e}"))?
            .text()
            .map_err(|e| format!("ScreenScraper request failed: {e}"))?;
        parse_genres_list(&xml)
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

    /// Exact lookup by disc serial — the identity that survives chd/rvz
    /// repacks, where file digests do not.
    pub fn screenscraper_serial_lookup(
        &self,
        creds: &ScraperCreds,
        serial: &str,
        platform_id: &str,
    ) -> Result<Vec<ScrapedGame>, String> {
        if !creds.is_configured() {
            return Err("ScreenScraper credentials not configured".to_string());
        }
        self.screenscraper_get(&serial_lookup_url(creds, serial, platform_id))
    }

    /// The account's quota counters. Best-effort: callers log failures.
    pub fn screenscraper_user_infos(&self, creds: &ScraperCreds) -> Result<SsUserInfos, String> {
        let body = self.http_get_text(&user_infos_url(creds))?;
        parse_user_infos(&body)
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

    /// The per-disc physical-media art for one game (`support-2D`), PNG
    /// bytes keyed by disc number, for the disc-picker tiles. `regions`
    /// is a preference order (the ROM's own region first); empty takes
    /// the default preference order. The jeuInfos answer is cached like
    /// the genre table — one request per game, ever. Images are never
    /// cached here: the caller persists them beside the game's other art.
    /// Discs the service has no art for are simply absent; the picker
    /// falls back to a numbered icon.
    pub fn screenscraper_disc_media(
        &self,
        creds: &ScraperCreds,
        ss_id: &str,
        regions: &[String],
    ) -> Result<Vec<DiscMedia>, String> {
        if !creds.is_configured() || ss_id.is_empty() {
            return Ok(Vec::new());
        }
        let scrapers = self.cache_dir.join("scrapers");
        let xml = self.cached_or_fetch(
            &scrapers,
            &format!("jeu_{ss_id}.xml"),
            &game_info_by_id_url(creds, ss_id),
        )?;
        let mut out = Vec::new();
        for (disc, url) in parse_disc_images(&xml, regions)? {
            // One missing image must not sink the discs that did arrive.
            match self.screenscraper_media(&url) {
                Ok(png) => out.push(DiscMedia { disc, png }),
                Err(e) => eprintln!("Disc art download failed (disc {disc}): {e}"),
            }
        }
        Ok(out)
    }

    /// The genre table, from the disk cache when present, fetched and
    /// cached otherwise — one request, ever, until the file is deleted.
    pub fn screenscraper_genres(&self, creds: &ScraperCreds) -> Result<Vec<GenreListEntry>, String> {
        if !creds.is_configured() {
            return Err("ScreenScraper credentials not configured".to_string());
        }
        let dir = self.cache_dir.join("scrapers");
        let body = self.cached_or_fetch(&dir, "ss_genres.xml", &genres_list_url(creds))?;
        parse_genres_list(&body)
    }

    /// The file's content when it exists, otherwise a fresh fetch written
    /// into it first. A failed cache write only costs the next fetch.
    fn cached_or_fetch(&self, dir: &std::path::Path, file: &str, url: &str) -> Result<String, String> {
        let path = dir.join(file);
        if let Ok(body) = std::fs::read_to_string(&path) {
            return Ok(body);
        }
        let body = self.http_get_text(url)?;
        if let Err(err) = std::fs::create_dir_all(dir).and_then(|_| std::fs::write(&path, &body)) {
            eprintln!("Failed to cache {file}: {err}");
        }
        Ok(body)
    }

    fn screenscraper_get(&self, url: &str) -> Result<Vec<ScrapedGame>, String> {
        let _s = tracing::info_span!("screenscraper_get", url = redact_url(url)).entered();
        // The service drops connections under load; one retry a moment
        // later saves the batch pass from a spurious failure. Status-level
        // answers are final and never retried.
        let (status, body) = match self.http.get(url).send() {
            Ok(resp) => {
                let status = resp.status();
                let body = resp
                    .text()
                    .map_err(|e| format!("ScreenScraper request failed: {e}"))?;
                (status, body)
            }
            Err(first) => {
                std::thread::sleep(std::time::Duration::from_millis(1500));
                let resp = self.http.get(url).send().map_err(|second| {
                    format!(
                        "ScreenScraper request failed: {} (and the retry: {})",
                        redact_text(&error_with_causes(&first), url),
                        redact_text(&error_with_causes(&second), url)
                    )
                })?;
                let status = resp.status();
                let body = resp
                    .text()
                    .map_err(|e| format!("ScreenScraper request failed: {e}"))?;
                (status, body)
            }
        };
        // A miss is a miss whatever the status line says: the exact
        // endpoints answer "Erreur : Rom/Iso/Dossier non trouvée !" with
        // an error status, and reading that as a failed request would
        // block the whole fallback chain.
        if is_rom_miss(&body) {
            return Ok(Vec::new());
        }
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

/// Both credential pairs ride in the query string, so no error message or
/// log line may carry it whole.
fn redact_url(url: &str) -> String {
    match url.split_once('?') {
        Some((base, _)) => base.to_string(),
        None => url.to_string(),
    }
}

fn redact_text(text: &str, url: &str) -> String {
    text.replace(url, &redact_url(url))
}

/// An error's Display plus its cause chain. reqwest's own text stops at
/// "error sending request for url (...)" — the cause underneath
/// ("connection refused", "timed out", a DNS failure) is what tells a
/// dead service from a dead network, so it rides along.
fn error_with_causes(err: &(impl std::error::Error + 'static)) -> String {
    let mut text = err.to_string();
    let mut cause = std::error::Error::source(err);
    while let Some(e) = cause {
        text.push_str(": ");
        text.push_str(&e.to_string());
        cause = e.source();
    }
    text
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
          <systeme id="18">Super Nintendo</systeme>
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
          <familles>
            <famille id="732" nom="Dragon Quest"/>
            <famille id="1234">Text-Named Series</famille>
          </familles>
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
        // Every region's name is kept for matching, with its region.
        assert!(game.names.len() >= 3);
        assert!(game.names.iter().any(|(r, n)| r == "us" && n == "Dragon Quest I & II"));
        assert!(game.names.iter().any(|(r, n)| r == "jp" && n.contains("ドラゴンクエスト")));
        assert_eq!(game.release_date, "1993-12-18");
        // The console the entry belongs to, named as the source spells it.
        assert_eq!(game.system_name, "Super Nintendo");
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
        // The series the source files the entry under, `nom` attribute
        // first and text content as the fallback spelling.
        assert_eq!(
            game.families
                .iter()
                .map(|f| (f.id.as_str(), f.name.as_str()))
                .collect::<Vec<_>>(),
            vec![("732", "Dragon Quest"), ("1234", "Text-Named Series")]
        );
        assert_eq!(game.players, "1-4");
        assert_eq!(game.rating, 18.0);
        // All three boards arrive as board + assigned value.
        assert_eq!(game.classifications.len(), 3);
        assert_eq!(game.classifications[1].kind, "PEGI");
        assert_eq!(game.classifications[1].value, "PEGI:3");
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
    fn test_parse_games_picks_region_and_disc_for_support_media() {
        // Shape captured from a live jeuInfos answer for Final Fantasy VII
        // (gameid 19249): per-disc media carry a `support` attribute,
        // single-support nodes carry none.
        let xml = r#"<Data><jeux><jeu id="19249">
            <noms><nom region="us">Final Fantasy VII</nom></noms>
            <medias>
              <media type="support-2D" region="eu" support="1">https://ss.example/ff7_eu1 .png</media>
              <media type="support-2D" region="us" support="1">https://ss.example/ff7_us1.png</media>
              <media type="support-2D" region="eu" support="3">https://ss.example/ff7_eu3.png</media>
              <media type="support-2D" region="eu" support="2">https://ss.example/ff7_eu2.png</media>
              <media type="support-texture" region="us" support="2">https://ss.example/ff7_tex2.png</media>
              <media type="box-2D" region="us">https://ss.example/ff7_box.png</media>
            </medias>
        </jeu></jeux></Data>"#;
        let game = parse_games(xml).unwrap().remove(0);
        // Every disc's art, sorted by number; us wins over eu for disc 1.
        assert_eq!(
            game.disc_images,
            vec![
                (1, "https://ss.example/ff7_us1.png".to_string()),
                (2, "https://ss.example/ff7_eu2.png".to_string()),
                (3, "https://ss.example/ff7_eu3.png".to_string()),
            ]
        );
    }

    #[test]
    fn test_parse_games_without_support_media_has_no_disc_images() {
        let game = parse_games(SEARCH_XML).unwrap().remove(0);
        assert!(game.disc_images.is_empty());
    }

    #[test]
    fn test_parse_disc_images_prefers_list_order_then_falls_back() {
        let xml = r#"<Data><jeux><jeu id="19249">
            <medias>
              <media type="support-2D" region="eu" support="1">https://ss.example/ff7_eu1.png</media>
              <media type="support-2D" region="us" support="1">https://ss.example/ff7_us1.png</media>
              <media type="support-2D" region="eu" support="2">https://ss.example/ff7_eu2.png</media>
            </medias>
        </jeu></jeux></Data>"#;
        // A multi-region list is a preference order: eu first even
        // though us exists, and us still serves disc 2 through the
        // fallback instead of dropping it.
        let ordered = ["eu".to_string(), "us".to_string(), "wor".to_string()];
        assert_eq!(
            parse_disc_images(xml, &ordered).unwrap(),
            vec![
                (1, "https://ss.example/ff7_eu1.png".to_string()),
                (2, "https://ss.example/ff7_eu2.png".to_string()),
            ]
        );
        // A single region pins exactly that region or nothing.
        let pinned = ["jp".to_string()];
        assert!(parse_disc_images(xml, &pinned).unwrap().is_empty());
    }

    const GENRES_XML: &str = r#"<Data>
      <genres>
        <genre>
          <id>10</id>
          <nom_fr>Action</nom_fr>
          <nom_en>Action</nom_en>
          <parent>0</parent>
          <medias><media type="background">https://x/y.jpg</media></medias>
        </genre>
        <genre>
          <id>2620</id>
          <nom_fr>Jeu de r&#244;le</nom_fr>
          <nom_en>Role Playing Game</nom_en>
          <parent>2600</parent>
        </genre>
      </genres>
    </Data>"#;

    #[test]
    fn test_parse_genres_list_prefers_english_names() {
        let genres = parse_genres_list(GENRES_XML).unwrap();
        assert_eq!(genres.len(), 2);
        assert_eq!(genres[0].id, "10");
        assert_eq!(genres[0].name, "Action");
        assert_eq!(genres[0].parent, "0");
        assert_eq!(genres[1].name, "Role Playing Game");
        assert_eq!(genres[1].parent, "2600");
        // Garbage answers surface as errors, not empty tables.
        assert!(parse_genres_list("<not closed").is_err());
    }

    #[test]
    fn test_genres_list_url_carries_credentials() {
        let creds = ScraperCreds {
            dev_id: "ira".into(),
            dev_password: "pw".into(),
            ..Default::default()
        };
        let url = genres_list_url(&creds);
        assert!(url.contains("genresListe.php?devid=ira&devpassword=pw"));
        assert!(url.contains("softname=ira"));
    }

    #[test]
    fn test_serial_lookup_url_carries_credentials_and_system() {
        let creds = ScraperCreds {
            dev_id: "ira".into(),
            dev_password: "pw".into(),
            ..Default::default()
        };
        let url = serial_lookup_url(&creds, "SLES-52005", "ps2");
        assert!(url.contains("jeuInfos.php?devid=ira&devpassword=pw"));
        assert!(url.contains("serialnum=SLES-52005"));
        assert!(url.contains("systemeid=58"));
    }

    #[test]
    fn test_parse_user_infos_reads_quota_counters() {
        let xml = r#"<Data><ssuser>
            <id>ayla</id><requeststoday>42</requeststoday>
            <maxrequestsperday>15000</maxrequestsperday>
            <requestskotoday>3</requestskotoday>
            <maxrequestskoperday>500</maxrequestskoperday>
            <maxrequestspermin>6</maxrequestspermin>
        </ssuser></Data>"#;
        let infos = parse_user_infos(xml).unwrap();
        assert_eq!(infos.requests_today, 42);
        assert_eq!(infos.max_requests_per_day, 15000);
        assert!(!infos.exhausted());
        let spent = SsUserInfos { requests_today: 15000, ..infos.clone() };
        assert!(spent.exhausted());
    }

    #[test]
    fn test_redact_url_strips_credentials_from_errors() {
        let url = "https://www.screenscraper.fr/api2/jeuRecherche.php?devid=a&devpassword=b&ssid=c&sspassword=d&recherche=x";
        let text = format!("ScreenScraper request failed: error sending request for url ({url})");
        let redacted = redact_text(&text, url);
        assert_eq!(
            redacted,
            "ScreenScraper request failed: error sending request for url (https://www.screenscraper.fr/api2/jeuRecherche.php)"
        );
        // A URL without a query passes through untouched.
        assert_eq!(redact_url("https://x/y.png"), "https://x/y.png");
    }

    #[test]
    fn test_error_with_causes_appends_the_whole_chain() {
        let inner = std::io::Error::new(std::io::ErrorKind::TimedOut, "connection timed out");
        let outer = std::io::Error::new(std::io::ErrorKind::ConnectionAborted, inner);
        let text = error_with_causes(&outer);
        assert!(
            text.contains("connection timed out"),
            "the cause must surface: {text}"
        );
        // A causeless error is just its own text.
        let bare = std::io::Error::other("dns failure");
        assert!(error_with_causes(&bare).contains("dns failure"));
    }

    #[test]
    fn test_parse_games_defrenchifies_the_picked_name() {
        let xml = r#"<Data><jeux><jeu id="1">
            <noms><nom region="us">007 : Everything or Nothing</nom></noms>
        </jeu></jeux></Data>"#;
        let games = parse_games(xml).unwrap();
        assert_eq!(games[0].name, "007: Everything or Nothing");
    }

    #[test]
    fn test_parse_games_handles_french_error_text() {
        assert!(parse_games("Erreur : Rom not found").unwrap().is_empty());
    }

    #[test]
    fn test_parse_games_reads_the_unwrapped_jeuinfos_shape() {
        // jeuInfos (md5 / serial / gameid) puts the single game straight
        // under <Data>, siblings and romid attribute included — exactly
        // the live serialnum=SLUS-20152 answer's shape.
        let xml = r#"<Data>
          <serveurs><cpu1>0</cpu1><threadsmin>4590</threadsmin></serveurs>
          <ssuser><requeststoday>20</requeststoday></ssuser>
          <jeu id="22425" romid="888779">
            <noms><nom region="us">Ace Combat 04 : Shattered Skies</nom></noms>
            <dates><date region="us">2001-11-01</date></dates>
          </jeu>
        </Data>"#;
        let games = parse_games(xml).unwrap();
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].ss_id, "22425");
        // The French spaced colon is normalized out of the picked name.
        assert_eq!(games[0].name, "Ace Combat 04: Shattered Skies");
        assert_eq!(games[0].release_date, "2001-11-01");
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

    fn scraped(ss_id: &str, name: &str) -> ScrapedGame {
        ScrapedGame {
            ss_id: ss_id.to_string(),
            name: name.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn test_sort_by_similarity_puts_the_exact_title_first() {
        let mut games = vec![
            scraped("3", "Portal Stories: Mel"),
            scraped("2", "Portal 2"),
            scraped("1", "Portal"),
        ];
        sort_by_similarity(&mut games, "Portal");
        assert_eq!(games[0].name, "Portal");
        assert_eq!(games[1].name, "Portal 2");
    }

    #[test]
    fn test_sort_by_similarity_folds_case_punctuation_and_accents() {
        let mut games = vec![scraped("1", "Pokemon XD: Gale of Darkness")];
        sort_by_similarity(&mut games, "  POKÉMON   XD! ");
        assert_eq!(games[0].name, "Pokemon XD: Gale of Darkness");
    }

    #[test]
    fn test_sort_by_similarity_scores_region_names_too() {
        let mut french_named = scraped("1", "Le Seigneur des Anneaux");
        french_named.names = vec![("us".to_string(), "The Lord of the Rings".to_string())];
        let mut games = vec![scraped("2", "Totally Different Game"), french_named];
        sort_by_similarity(&mut games, "lord of the rings");
        assert_eq!(games[0].ss_id, "1");
    }

    #[test]
    fn test_sort_by_similarity_keeps_order_when_nothing_matches() {
        let mut games = vec![scraped("1", "Aaa"), scraped("2", "Bbb")];
        sort_by_similarity(&mut games, "Zzzz");
        assert_eq!(games[0].ss_id, "1");
        assert_eq!(games[1].ss_id, "2");
    }
}
